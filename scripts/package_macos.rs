mod package_common;

use std::path::Path;
use std::process::Command;

use package_common::{BuildEnvVars, EnvConfig, PackageTarget};

const ENV_APPLE_SIGNING_IDENTITY: &str = "APPLE_SIGNING_IDENTITY";
const ENV_APPLE_API_ISSUER: &str = "APPLE_API_ISSUER";
const ENV_APPLE_API_KEY: &str = "APPLE_API_KEY";
const ENV_APPLE_API_KEY_PATH: &str = "APPLE_API_KEY_PATH";
const ENV_APPLE_ID: &str = "APPLE_ID";
const ENV_APPLE_PASSWORD: &str = "APPLE_PASSWORD";
const ENV_APPLE_TEAM_ID: &str = "APPLE_TEAM_ID";

fn main() {
    if let Err(message) = package_common::run(
        PackageTarget::Mac,
        load_macos_build_env,
        print_macos_help,
    ) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn load_macos_build_env(env_config: &EnvConfig) -> Result<BuildEnvVars, String> {
    let mut values = Vec::new();
    let signing_identity = required_env(env_config, ENV_APPLE_SIGNING_IDENTITY)?;
    if signing_identity.trim() == "-" {
        return Err("macOS 发布包不能使用 ad-hoc 签名 identity '-'。请使用 Developer ID Application 证书。".to_string());
    }
    validate_signing_identity(&signing_identity)?;
    values.push((ENV_APPLE_SIGNING_IDENTITY.to_string(), signing_identity));
    append_notarization_env(env_config, &mut values)?;
    Ok(values)
}

fn append_notarization_env(
    env_config: &EnvConfig,
    values: &mut BuildEnvVars,
) -> Result<(), String> {
    let api_issuer = env_config.get(ENV_APPLE_API_ISSUER);
    let api_key = env_config.get(ENV_APPLE_API_KEY);
    let api_key_path = env_config.absolute_path(ENV_APPLE_API_KEY_PATH)?;
    if let (Some(api_issuer), Some(api_key), Some(api_key_path)) =
        (api_issuer, api_key, api_key_path)
    {
        values.push((ENV_APPLE_API_ISSUER.to_string(), api_issuer));
        values.push((ENV_APPLE_API_KEY.to_string(), api_key));
        values.push((
            ENV_APPLE_API_KEY_PATH.to_string(),
            path_to_env_value(&api_key_path)?,
        ));
        return Ok(());
    }

    let apple_id = env_config.get(ENV_APPLE_ID);
    let apple_password = env_config.get(ENV_APPLE_PASSWORD);
    let apple_team_id = env_config.get(ENV_APPLE_TEAM_ID);
    if let (Some(apple_id), Some(apple_password), Some(apple_team_id)) =
        (apple_id, apple_password, apple_team_id)
    {
        values.push((ENV_APPLE_ID.to_string(), apple_id));
        values.push((ENV_APPLE_PASSWORD.to_string(), apple_password));
        values.push((ENV_APPLE_TEAM_ID.to_string(), apple_team_id));
        return Ok(());
    }

    Err(format!(
        "缺少 macOS notarization 配置。请在 .env 或环境变量中配置 App Store Connect API: {}, {}, {}，或 Apple ID: {}, {}, {}。",
        ENV_APPLE_API_ISSUER,
        ENV_APPLE_API_KEY,
        ENV_APPLE_API_KEY_PATH,
        ENV_APPLE_ID,
        ENV_APPLE_PASSWORD,
        ENV_APPLE_TEAM_ID
    ))
}

fn required_env(env_config: &EnvConfig, key: &str) -> Result<String, String> {
    env_config
        .get(key)
        .ok_or_else(|| format!("缺少 {key}。请在 .env 或环境变量中设置。"))
}

fn validate_signing_identity(identity: &str) -> Result<(), String> {
    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .map_err(|error| format!("检查 macOS 代码签名证书失败: {error}"))?;

    if !output.status.success() {
        return Err("检查 macOS 代码签名证书失败，请确认 Xcode Command Line Tools 和钥匙串可用。".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains(identity) {
        return Ok(());
    }

    Err(format!(
        "本机 codesigning identity 中未找到 {identity}。请执行 security find-identity -v -p codesigning 确认 APPLE_SIGNING_IDENTITY。"
    ))
}

fn path_to_env_value(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| format!("路径不是有效 UTF-8: {}", path.display()))
}

fn print_macos_help() {
    println!(
        "macOS Apple 签名和公证配置:
  APPLE_SIGNING_IDENTITY=Developer ID Application: Your Name (TEAMID)

  推荐使用 App Store Connect API:
  APPLE_API_ISSUER=<issuer-id>
  APPLE_API_KEY=<key-id>
  APPLE_API_KEY_PATH=/absolute/path/AuthKey_<key-id>.p8

  或使用 Apple ID:
  APPLE_ID=<apple-id-email>
  APPLE_PASSWORD=<app-specific-password>
  APPLE_TEAM_ID=<team-id>
"
    );
}
