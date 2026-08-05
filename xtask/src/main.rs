use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const HAIR_REPOSITORY: &str = "https://github.com/Alan-Jowett/HardwareAbstractionIR.git";
const HAIR_REVISION: &str = "60e0de038210008018d0168f45854d113a5964cc";
const HAIR_INPUT_URL: &str = "https://raw.githubusercontent.com/Alan-Jowett/HardwareAbstractionIR/60e0de038210008018d0168f45854d113a5964cc/evidence/wch/ch32v203g6u6/hair.json";
const FLASH_LIMIT: u64 = 32 * 1024;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("generate-hal") => match generate_hal() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("HAL generation failed: {error}");
                ExitCode::FAILURE
            }
        },
        Some("size") => match report_size() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("size report failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask generate-hal|size");
            ExitCode::FAILURE
        }
    }
}

fn report_size() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace root not found")?
        .to_path_buf();
    let target = root
        .join("target")
        .join("riscv32imc-unknown-none-elf")
        .join("release");
    for name in ["optibridge", "i2c-bridge"] {
        let image = target.join(name);
        if !image.is_file() {
            return Err(format!("missing release image {}", image.display()).into());
        }
        let output = Command::new("llvm-size").arg(&image).output()?;
        if !output.status.success() {
            return Err("llvm-size returned a failure".into());
        }
        let report = String::from_utf8(output.stdout)?;
        let fields = report
            .lines()
            .nth(1)
            .ok_or("llvm-size returned no size row")?
            .split_whitespace()
            .collect::<Vec<_>>();
        let text = fields
            .first()
            .ok_or("llvm-size row has no text field")?
            .parse::<u64>()?;
        let data = fields
            .get(1)
            .ok_or("llvm-size row has no data field")?
            .parse::<u64>()?;
        let flash = text + data;
        println!("{name}: text={text} bytes, data={data} bytes, flash={flash} bytes");
        if flash > FLASH_LIMIT {
            return Err(format!("{name} exceeds the {FLASH_LIMIT}-byte flash limit").into());
        }
    }
    Ok(())
}

fn generate_hal() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace root not found")?
        .to_path_buf();
    let generated = root.join(".generated").join("ch32v203g6u6");
    let input = generated.join("hair.json");
    let source = root.join(".generated").join("hardware-abstraction-ir");

    fs::create_dir_all(&generated)?;
    download(HAIR_INPUT_URL, &input)?;

    if !source.join("Cargo.toml").is_file() {
        let clone_status = Command::new("git")
            .args(["clone", "--quiet", HAIR_REPOSITORY])
            .arg(&source)
            .status()?;
        if !clone_status.success() {
            return Err("failed to clone upstream HAIR repository".into());
        }
    }

    let checkout_status = Command::new("git")
        .args([
            "-C",
            source.to_str().ok_or("invalid source path")?,
            "checkout",
            "--quiet",
            HAIR_REVISION,
        ])
        .status()?;
    if !checkout_status.success() {
        return Err("failed to select pinned upstream HAIR revision".into());
    }

    let status = Command::new("cargo")
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(source.join("Cargo.toml"))
        .args(["--", "generate", "embassy"])
        .arg(&input)
        .arg("--output-dir")
        .arg(generated.join("embassy"))
        .current_dir(&source)
        .status()?;
    if !status.success() {
        return Err("upstream HAIR generator returned a failure".into());
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            url,
            "--output",
        ])
        .arg(destination)
        .status()?;
    if !status.success() {
        return Err(format!("failed to download {url}").into());
    }
    Ok(())
}
