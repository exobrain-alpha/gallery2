mod package_common;

use package_common::{BuildEnvVars, EnvConfig, PackageTarget};

fn main() {
    if let Err(message) = package_common::run(
        PackageTarget::Windows,
        load_windows_build_env,
        print_windows_help,
    ) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn load_windows_build_env(_env_config: &EnvConfig) -> Result<BuildEnvVars, String> {
    Ok(Vec::new())
}

fn print_windows_help() {
    println!(
        "Windows 平台配置:
  当前只生成 NSIS 安装包和 updater 签名产物；暂不接入 Windows 代码签名证书。
"
    );
}
