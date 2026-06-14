use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

const ENV_FILE_NAME: &str = ".env";
const ENV_RELEASES_ROOT_URL: &str = "GALLERY_RELEASES_ROOT_URL";
const ENV_SIGNING_KEY_PATH: &str = "GALLERY_SIGNING_KEY_PATH";

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse(env::args().skip(1).collect())?;
    if args.help {
        print_help();
        return Ok(());
    }

    let repo_root = find_repo_root()?;
    let env_config = EnvConfig::load(&repo_root)?;
    let config_path = repo_root.join("src-tauri").join("tauri.conf.json");
    let mut config_text = fs::read_to_string(&config_path)
        .map_err(|error| format!("读取 Tauri 配置失败: {} ({error})", config_path.display()))?;
    let mut config = AppConfig::from_json(&config_text)?;

    if !args.dry_run {
        if let Some(version) = prompt_version(&config.version)? {
            config_text = replace_json_string_field(&config_text, "version", &version)
                .ok_or_else(|| "更新 src-tauri/tauri.conf.json version 失败。".to_string())?;
            fs::write(&config_path, &config_text).map_err(|error| {
                format!("写入 Tauri 配置失败: {} ({error})", config_path.display())
            })?;
            config.version = version;
            println!("版本号已更新为 {}", config.version);
        }
    }

    let private_key_path = env_config.absolute_path(ENV_SIGNING_KEY_PATH)?;

    let missing_public_key = config.pubkey.trim().is_empty();
    let missing_private_key = private_key_path
        .as_ref()
        .map(|path| !path.is_file())
        .unwrap_or(true);
    let empty_private_key = private_key_path
        .as_ref()
        .filter(|path| path.is_file())
        .map(|path| {
            fs::read_to_string(path)
                .map(|content| content.trim().is_empty())
                .unwrap_or(true)
        })
        .unwrap_or(false);

    if missing_public_key || missing_private_key || empty_private_key {
        print_key_configuration_hint(
            &repo_root,
            &config_path,
            private_key_path.as_deref(),
            missing_public_key,
            missing_private_key,
            empty_private_key,
        );
        return Err("未检测到完整的更新包签名公私钥配置，已退出。".to_string());
    }
    let private_key_path = private_key_path
        .ok_or_else(|| format!("请在 {ENV_FILE_NAME} 或环境变量中设置 {ENV_SIGNING_KEY_PATH}。"))?;

    if !config.create_updater_artifacts {
        return Err(
            "src-tauri/tauri.conf.json 的 bundle.createUpdaterArtifacts 未启用，Tauri 不会生成 updater 签名产物。"
                .to_string(),
        );
    }

    let platform = Platform::current()?;
    let release_base_url = match args.base_url {
        Some(base_url) => base_url,
        None => {
            let release_root_url = env_config
                .get(ENV_RELEASES_ROOT_URL)
                .or_else(|| config.release_base_url())
                .ok_or_else(|| {
                    format!(
                        "请通过 --base-url、{ENV_FILE_NAME} 的 {ENV_RELEASES_ROOT_URL}，或 updater endpoint 配置产物 URL 前缀。"
                    )
                })?;
            format!(
                "{}/{}",
                trim_trailing_slashes(&release_root_url),
                config.version
            )
        }
    };
    let output_path = args
        .out
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .unwrap_or_else(|| {
            repo_root
                .join("src-tauri")
                .join("target")
                .join("release")
                .join("bundle")
                .join("latest.json")
        });

    let tauri_script = repo_root
        .join("node_modules")
        .join("@tauri-apps")
        .join("cli")
        .join("tauri.js");
    if !tauri_script.is_file() {
        return Err(format!(
            "未找到本地 Tauri CLI: {}\n请先执行 npm install。",
            tauri_script.display()
        ));
    }

    let build_steps = platform.build_steps();
    if args.dry_run {
        println!("仓库: {}", repo_root.display());
        println!("私钥: {}", private_key_path.display());
        println!("平台: {}", platform.platform_key());
        for step in &build_steps {
            println!("将执行: node {} {}", tauri_script.display(), step.join(" "));
        }
        println!("将生成: {}", output_path.display());
        println!("产物 URL 前缀: {release_base_url}");
        return Ok(());
    }

    let password = prompt_password()?;
    let private_key = fs::read_to_string(&private_key_path)
        .map_err(|error| format!("读取私钥失败: {} ({error})", private_key_path.display()))?;
    let private_key = private_key.trim().to_string();

    for step in &build_steps {
        run_tauri_build(&repo_root, &tauri_script, step, &private_key, &password)?;
    }

    let artifact = find_updater_artifact(&repo_root, platform, &config.version)?;
    write_latest_json(
        &output_path,
        &config,
        platform,
        &artifact,
        &release_base_url,
    )?;

    println!("已更新 {}", output_path.display());
    println!(
        "{}: {}",
        platform.platform_key(),
        artifact.url(&release_base_url)
    );
    Ok(())
}

#[derive(Debug)]
struct EnvConfig {
    values: HashMap<String, String>,
}

impl EnvConfig {
    fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join(ENV_FILE_NAME);
        let values = if path.is_file() {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("读取 env 文件失败: {} ({error})", path.display()))?;
            parse_env_file(&source)?
        } else {
            HashMap::new()
        };

        Ok(EnvConfig { values })
    }

    fn get(&self, key: &str) -> Option<String> {
        env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.values
                    .get(key)
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
    }

    fn absolute_path(&self, key: &str) -> Result<Option<PathBuf>, String> {
        self.get(key)
            .map(|value| parse_absolute_env_path(key, &value))
            .transpose()
    }
}

fn parse_absolute_env_path(key: &str, value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "{key} 只支持绝对路径，请使用 /Users/...、C:\\... 或 UNC 路径，不要使用相对路径或 ~。"
        ))
    }
}

fn parse_env_file(source: &str) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();
    for (line_index, raw_line) in source.lines().enumerate() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim_start();
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!(
                "{} 第 {} 行缺少 '='。",
                ENV_FILE_NAME,
                line_index + 1
            ));
        };
        let key = key.trim();
        if key.is_empty()
            || !key.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
        {
            return Err(format!(
                "{} 第 {} 行变量名无效: {key}",
                ENV_FILE_NAME,
                line_index + 1
            ));
        }
        values.insert(key.to_string(), parse_env_value(value));
    }
    Ok(values)
}

fn parse_env_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let quote = value.as_bytes()[0];
        if matches!(quote, b'\'' | b'"') && value.as_bytes().last() == Some(&quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[derive(Debug)]
struct Args {
    base_url: Option<String>,
    dry_run: bool,
    help: bool,
    out: Option<PathBuf>,
}

impl Args {
    fn parse(argv: Vec<String>) -> Result<Self, String> {
        let mut args = Args {
            base_url: None,
            dry_run: false,
            help: false,
            out: None,
        };

        let mut index = 0;
        while index < argv.len() {
            let token = &argv[index];
            match token.as_str() {
                "--help" | "-h" => args.help = true,
                "--dry-run" => args.dry_run = true,
                "--base-url" => {
                    index += 1;
                    args.base_url = Some(read_option_value(&argv, index, "--base-url")?);
                }
                "--out" => {
                    index += 1;
                    args.out = Some(PathBuf::from(read_option_value(&argv, index, "--out")?));
                }
                _ if token.starts_with("--base-url=") => {
                    args.base_url = Some(token["--base-url=".len()..].to_string());
                }
                _ if token.starts_with("--out=") => {
                    args.out = Some(PathBuf::from(token["--out=".len()..].to_string()));
                }
                _ => return Err(format!("未知参数: {token}")),
            }
            index += 1;
        }

        Ok(args)
    }
}

#[derive(Clone, Copy, Debug)]
enum Platform {
    Linux,
    Mac,
    Windows,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match env::consts::OS {
            "linux" => Ok(Platform::Linux),
            "macos" => Ok(Platform::Mac),
            "windows" => Ok(Platform::Windows),
            other => Err(format!("不支持的打包平台: {other}")),
        }
    }

    fn build_steps(self) -> Vec<Vec<&'static str>> {
        match self {
            Platform::Linux => vec![vec!["build", "--bundles", "appimage"]],
            Platform::Mac => vec![vec!["build", "--bundles", "app,dmg"]],
            Platform::Windows => vec![vec!["build", "--bundles", "nsis"]],
        }
    }

    fn platform_key(self) -> String {
        let os = match self {
            Platform::Linux => "linux",
            Platform::Mac => "darwin",
            Platform::Windows => "windows",
        };
        format!("{os}-{}", updater_arch())
    }
}

#[derive(Debug)]
struct AppConfig {
    create_updater_artifacts: bool,
    endpoint: Option<String>,
    product_name: String,
    pubkey: String,
    version: String,
}

impl AppConfig {
    fn from_json(source: &str) -> Result<Self, String> {
        let version = json_string_field(source, "version")
            .ok_or_else(|| "src-tauri/tauri.conf.json 缺少 version。".to_string())?;
        let product_name =
            json_string_field(source, "productName").unwrap_or_else(|| "Gallery".to_string());
        let pubkey = json_string_field(source, "pubkey").unwrap_or_default();
        let endpoint = json_first_array_string(source, "endpoints");
        let create_updater_artifacts = json_truthy_field(source, "createUpdaterArtifacts");

        Ok(AppConfig {
            create_updater_artifacts,
            endpoint,
            product_name,
            pubkey,
            version,
        })
    }

    fn release_base_url(&self) -> Option<String> {
        let endpoint = self.endpoint.as_ref()?;
        if endpoint.contains("{{") {
            return None;
        }
        endpoint
            .strip_suffix("/latest.json")
            .map(|value| value.to_string())
    }
}

#[derive(Debug)]
struct UpdaterArtifact {
    path: PathBuf,
    signature_path: PathBuf,
}

impl UpdaterArtifact {
    fn url(&self, release_base_url: &str) -> String {
        let file_name = self
            .path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        format!(
            "{}/{}",
            trim_trailing_slashes(release_base_url),
            url_path_segment(file_name)
        )
    }
}

fn run_tauri_build(
    repo_root: &Path,
    tauri_script: &Path,
    args: &[&str],
    private_key: &str,
    password: &str,
) -> Result<(), String> {
    println!("执行: node {} {}", tauri_script.display(), args.join(" "));
    let status = Command::new("node")
        .arg(tauri_script)
        .args(args)
        .current_dir(repo_root)
        .env("TAURI_SIGNING_PRIVATE_KEY", private_key)
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", password)
        .status()
        .map_err(|error| format!("启动 Tauri 构建失败: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("Tauri 构建失败，退出码: {status}"))
    }
}

fn find_updater_artifact(
    repo_root: &Path,
    platform: Platform,
    version: &str,
) -> Result<UpdaterArtifact, String> {
    let bundle_root = repo_root
        .join("src-tauri")
        .join("target")
        .join("release")
        .join("bundle");

    let candidates = match platform {
        Platform::Mac => collect_candidates(&bundle_root.join("macos"), &[".app.tar.gz.sig"])?,
        Platform::Linux => collect_candidates(
            &bundle_root.join("appimage"),
            &[".AppImage.sig", ".AppImage.tar.gz.sig"],
        )?,
        Platform::Windows => {
            collect_candidates(&bundle_root.join("nsis"), &[".exe.sig", ".nsis.zip.sig"])?
        }
    };

    choose_latest_candidate(candidates, version).ok_or_else(|| {
        format!(
            "未找到当前平台 updater 产物签名，请检查 {} 下是否存在对应 .sig 文件。",
            bundle_root.display()
        )
    })
}

fn collect_candidates(dir: &Path, suffixes: &[&str]) -> Result<Vec<UpdaterArtifact>, String> {
    let mut candidates = Vec::new();
    collect_candidates_recursive(dir, suffixes, &mut candidates)?;
    Ok(candidates)
}

fn collect_candidates_recursive(
    dir: &Path,
    suffixes: &[&str],
    candidates: &mut Vec<UpdaterArtifact>,
) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in
        fs::read_dir(dir).map_err(|error| format!("读取目录失败: {} ({error})", dir.display()))?
    {
        let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_candidates_recursive(&path, suffixes, candidates)?;
            continue;
        }

        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !suffixes.iter().any(|suffix| name.ends_with(suffix)) {
            continue;
        }

        let Some(artifact_path) = strip_sig_extension(&path) else {
            continue;
        };
        if artifact_path.is_file() {
            candidates.push(UpdaterArtifact {
                path: artifact_path,
                signature_path: path,
            });
        }
    }

    Ok(())
}

fn choose_latest_candidate(
    mut candidates: Vec<UpdaterArtifact>,
    version: &str,
) -> Option<UpdaterArtifact> {
    candidates.sort_by(|left, right| {
        let left_contains_version = file_name_contains(&left.path, version);
        let right_contains_version = file_name_contains(&right.path, version);
        right_contains_version
            .cmp(&left_contains_version)
            .then_with(|| modified_time(&right.path).cmp(&modified_time(&left.path)))
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.into_iter().next()
}

fn write_latest_json(
    output_path: &Path,
    config: &AppConfig,
    platform: Platform,
    artifact: &UpdaterArtifact,
    release_base_url: &str,
) -> Result<(), String> {
    let signature = fs::read_to_string(&artifact.signature_path).map_err(|error| {
        format!(
            "读取签名失败: {} ({error})",
            artifact.signature_path.display()
        )
    })?;
    let signature = signature.trim();
    if signature.is_empty() {
        return Err(format!(
            "签名文件为空: {}",
            artifact.signature_path.display()
        ));
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建输出目录失败: {} ({error})", parent.display()))?;
    }

    let platform_key = platform.platform_key();
    let latest_json = format!(
        "{{\n  \"version\": \"{}\",\n  \"notes\": \"{} {}\",\n  \"platforms\": {{\n    \"{}\": {{\n      \"signature\": \"{}\",\n      \"url\": \"{}\"\n    }}\n  }}\n}}\n",
        json_escape(&config.version),
        json_escape(&config.product_name),
        json_escape(&config.version),
        json_escape(&platform_key),
        json_escape(signature),
        json_escape(&artifact.url(release_base_url)),
    );

    fs::write(output_path, latest_json)
        .map_err(|error| format!("写入 latest 文件失败: {} ({error})", output_path.display()))
}

fn prompt_version(current_version: &str) -> Result<Option<String>, String> {
    loop {
        print!("请输入版本号 [当前: {current_version}]: ");
        io::stdout()
            .flush()
            .map_err(|error| format!("刷新终端输出失败: {error}"))?;

        let mut version = String::new();
        io::stdin()
            .read_line(&mut version)
            .map_err(|error| format!("读取版本号失败: {error}"))?;
        let version = version.trim();
        if version.is_empty() {
            return Ok(None);
        }
        if !is_semver_version(version) {
            println!("版本号必须是 semver 格式，例如 0.2.1");
            continue;
        }
        return Ok(Some(version.to_string()));
    }
}

fn prompt_password() -> Result<String, String> {
    loop {
        print!("请输入私钥密码（明文输入）: ");
        io::stdout()
            .flush()
            .map_err(|error| format!("刷新终端输出失败: {error}"))?;

        let mut password = String::new();
        let bytes_read = io::stdin()
            .read_line(&mut password)
            .map_err(|error| format!("读取私钥密码失败: {error}"))?;
        if bytes_read == 0 {
            return Err("未读取到私钥密码。".to_string());
        }
        let password = password.trim_end_matches(['\r', '\n']).to_string();
        if password.trim().is_empty() {
            println!("密码不能为空");
            continue;
        }
        return Ok(password);
    }
}

fn is_semver_version(value: &str) -> bool {
    let Some((core, suffix)) = split_semver_suffix(value) else {
        return false;
    };
    let mut parts = core.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    if !is_semver_number(major) || !is_semver_number(minor) || !is_semver_number(patch) {
        return false;
    }
    suffix.map(is_semver_suffix).unwrap_or(true)
}

fn split_semver_suffix(value: &str) -> Option<(&str, Option<&str>)> {
    let first_suffix = value.find('-').into_iter().chain(value.find('+')).min();
    match first_suffix {
        Some(index) if index > 0 => Some((&value[..index], Some(&value[index..]))),
        Some(_) => None,
        None => Some((value, None)),
    }
}

fn is_semver_number(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| character.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn is_semver_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !matches!(bytes[0], b'-' | b'+') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'+'))
        && !value.ends_with(['-', '.', '+'])
}

fn find_repo_root() -> Result<PathBuf, String> {
    if let Ok(current_dir) = env::current_dir() {
        if let Some(root) = find_repo_root_from(&current_dir) {
            return Ok(root);
        }
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            if let Some(root) = find_repo_root_from(parent) {
                return Ok(root);
            }
        }
    }

    Err("无法定位仓库根目录，请在 gallery2 仓库内运行本脚本。".to_string())
}

fn find_repo_root_from(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        if dir.join("package.json").is_file()
            && dir.join("src-tauri").join("tauri.conf.json").is_file()
        {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn print_key_configuration_hint(
    repo_root: &Path,
    config_path: &Path,
    private_key_path: Option<&Path>,
    missing_public_key: bool,
    missing_private_key: bool,
    empty_private_key: bool,
) {
    println!("未检测到完整的 Tauri updater 签名公私钥配置。");
    if missing_private_key {
        if let Some(path) = private_key_path {
            println!("缺少私钥文件: {}", path.display());
        } else {
            println!(
                "缺少私钥路径配置: 请在 {} 或环境变量中设置 {}，且必须使用绝对路径。",
                ENV_FILE_NAME, ENV_SIGNING_KEY_PATH
            );
        }
    } else if empty_private_key {
        if let Some(path) = private_key_path {
            println!("私钥文件为空: {}", path.display());
        }
    }
    if missing_public_key {
        println!(
            "缺少公钥配置: {} 的 plugins.updater.pubkey",
            config_path.display()
        );
    }
    println!("请在仓库内执行 package.json 已配置命令生成密钥:");
    println!("  npm run signer:generate");
    println!(
        "然后把命令输出的 public key 写入 src-tauri/tauri.conf.json 的 plugins.updater.pubkey，并把私钥路径写入 {} 的 {}。", ENV_FILE_NAME, ENV_SIGNING_KEY_PATH
    );
    println!("仓库: {}", repo_root.display());
}

fn updater_arch() -> &'static str {
    match env::consts::ARCH {
        "aarch64" => "aarch64",
        "arm" => "armv7",
        "x86" => "i686",
        "x86_64" => "x86_64",
        _ => env::consts::ARCH,
    }
}

fn strip_sig_extension(path: &Path) -> Option<PathBuf> {
    let value = path.to_string_lossy();
    value.strip_suffix(".sig").map(PathBuf::from)
}

fn file_name_contains(path: &Path, needle: &str) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(|value| value.contains(needle))
        .unwrap_or(false)
}

fn modified_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn trim_trailing_slashes(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

fn read_option_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    let Some(value) = argv.get(index) else {
        return Err(format!("缺少 {name} 的参数值"));
    };
    if value.starts_with("--") {
        return Err(format!("缺少 {name} 的参数值"));
    }
    Ok(value.to_string())
}

fn json_truthy_field(source: &str, key: &str) -> bool {
    let Some(start) = json_value_start(source, key) else {
        return false;
    };
    let rest = source[start..].trim_start();
    rest.starts_with("true") || rest.starts_with("\"v1Compatible\"")
}

fn json_first_array_string(source: &str, key: &str) -> Option<String> {
    let start = json_value_start(source, key)?;
    let bytes = source.as_bytes();
    let mut index = skip_ws(bytes, start);
    if bytes.get(index) != Some(&b'[') {
        return None;
    }
    index = skip_ws(bytes, index + 1);
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    parse_json_string_at(source, index).map(|(value, _)| value)
}

fn json_string_field(source: &str, key: &str) -> Option<String> {
    let start = json_value_start(source, key)?;
    let index = skip_ws(source.as_bytes(), start);
    if source.as_bytes().get(index) != Some(&b'"') {
        return None;
    }
    parse_json_string_at(source, index).map(|(value, _)| value)
}

fn replace_json_string_field(source: &str, key: &str, value: &str) -> Option<String> {
    let start = json_value_start(source, key)?;
    let start = skip_ws(source.as_bytes(), start);
    if source.as_bytes().get(start) != Some(&b'"') {
        return None;
    }
    let (_, end) = parse_json_string_at(source, start)?;
    let mut output = String::new();
    output.push_str(&source[..start]);
    output.push('"');
    output.push_str(&json_escape(value));
    output.push('"');
    output.push_str(&source[end..]);
    Some(output)
}

fn json_value_start(source: &str, key: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }

        let (field, next_index) = parse_json_string_at(source, index)?;
        index = next_index;
        if field != key {
            continue;
        }

        let colon_index = skip_ws(bytes, next_index);
        if bytes.get(colon_index) != Some(&b':') {
            continue;
        }
        return Some(skip_ws(bytes, colon_index + 1));
    }
    None
}

fn parse_json_string_at(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }

    let mut output = String::new();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => return Some((output, index + 1)),
            b'\\' => {
                index += 1;
                let escaped = *bytes.get(index)?;
                match escaped {
                    b'"' => output.push('"'),
                    b'\\' => output.push('\\'),
                    b'/' => output.push('/'),
                    b'b' => output.push('\u{0008}'),
                    b'f' => output.push('\u{000c}'),
                    b'n' => output.push('\n'),
                    b'r' => output.push('\r'),
                    b't' => output.push('\t'),
                    b'u' => {
                        let hex_start = index + 1;
                        let hex_end = hex_start + 4;
                        let hex = source.get(hex_start..hex_end)?;
                        let value = u16::from_str_radix(hex, 16).ok()?;
                        output.push(char::from_u32(u32::from(value))?);
                        index = hex_end - 1;
                    }
                    _ => return None,
                }
            }
            byte => output.push(byte as char),
        }
        index += 1;
    }

    None
}

fn skip_ws(bytes: &[u8], mut index: usize) -> usize {
    while let Some(byte) = bytes.get(index) {
        if !matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
            break;
        }
        index += 1;
    }
    index
}

fn json_escape(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            c if c.is_control() => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output
}

fn url_path_segment(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                output.push(byte as char);
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

fn print_help() {
    println!(
        "Gallery updater packaging helper

Usage:
  npm run package:compile
  ./gallery2_package
  .\\gallery2_package.exe

Options:
  --base-url <url>  覆盖产物 URL 前缀；未传入时从 env release 根 URL 或 updater endpoint 推导到当前版本目录。
  --out <path>      覆盖 latest.json 输出路径。
  --dry-run         只检查配置并打印将执行的构建命令。
  --help            显示帮助。

env 配置:
  GALLERY_SIGNING_KEY_PATH=<absolute-path>
  GALLERY_RELEASES_ROOT_URL=https://example.com/releases

说明:
  GALLERY_SIGNING_KEY_PATH 只支持绝对路径，不支持相对路径或 ~。

签名配置:
  私钥: .env -> GALLERY_SIGNING_KEY_PATH
  公钥: src-tauri/tauri.conf.json -> plugins.updater.pubkey
"
    );
}
