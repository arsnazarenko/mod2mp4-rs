use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::{ArgGroup, Args, Parser};
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::download::auto_download;

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct Input {
    #[arg(short = 'f', long = "file")]
    file: Option<String>,

    #[arg(short = 'd', long = "dir")]
    dir: Option<String>,
}

#[derive(Parser)]
#[command(name = "mod2mp4")]
#[command(group(
    ArgGroup::new("input")
        .required(true)
        .multiple(false)
        .args(["file", "dir"])
))]
struct Cli {
    #[clap(flatten)]
    input: Input,

    #[arg(short = 'o', long = "output")]
    output: String,

    #[arg(long = "date", default_value_t = false)]
    date: bool,
}

fn convert_mod_to_mp4(num: usize, input: &Path, output: &Path) -> io::Result<()> {
    let filter = "yadif=0:-1:0,minterpolate=fps=50:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1";

    print!(
        "{}. Converting {} -> {} ... ",
        num,
        input.display(),
        output.display()
    );
    let _ = io::stdout().flush();
    let mut cmd = FfmpegCommand::new();
    cmd.input(input.to_str().unwrap())
        .codec_video("libx264")
        .preset("veryslow")
        .crf(17)
        .args(["-filter:v", filter])
        .pix_fmt("yuv420p")
        .codec_audio("aac")
        .args(["-b:a", "384k"])
        .args(["-movflags", "+faststart"])
        .overwrite()
        .output(output.to_str().unwrap());

    let mut child = cmd.spawn()?;
    let mut stderr_str = String::new();
    if let Some(mut stderr) = child.take_stderr() {
        let _ = stderr.read_to_string(&mut stderr_str);
    }

    let ret = child.wait()?;

    if !ret.success() {
        println!("Error");
        let err_msg = format!(
            "FFmpeg process failed with reture code: {}
            Stderr:\n{}",
            ret.code().unwrap(),
            stderr_str,
        );

        Err(io::Error::other(err_msg))
    } else {
        println!("Success");
        Ok(())
    }
}

fn process_file(num: usize, input: &Path, out_dir: &Path, date: bool) -> io::Result<()> {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    let moi_date = if date {
        let moi_file = input.with_extension("MOI");
        let moi_date = parse_moi_date(moi_file.clone()).map_err(|e| {
            io::Error::other(format!("Failed to open {} file: {}", moi_file.display(), e))
        })?;
        moi_date.to_string()
    } else {
        "".to_string()
    };

    let output = out_dir.join(format!("{}_{}.mp4", stem, moi_date));

    convert_mod_to_mp4(num, input, &output)
}

enum DatePart {
    Year,
    Month,
    Day,
    Hour,
    Minute,
}

impl DatePart {
    fn get_range(&self) -> std::ops::Range<usize> {
        match self {
            DatePart::Year => 6..8,
            DatePart::Month => 8..9,
            DatePart::Day => 9..10,
            DatePart::Hour => 10..11,
            DatePart::Minute => 11..12,
        }
    }
}

struct MoiDate {
    year: u16,
    month: u8,
    day: u8,
    hours: u8,
    minutes: u8,
}

impl std::fmt::Display for MoiDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}_{:02}_{:02}",
            self.year, self.month, self.day, self.hours, self.minutes
        )
    }
}

fn parse_moi_date(path: impl AsRef<Path>) -> io::Result<MoiDate> {
    let mut buf = vec![];
    let mut file = std::fs::File::open(path)?;
    file.read_to_end(&mut buf)?;
    if buf.len() <= 12 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Moi file size less then minimum required",
        ));
    }

    let year = u16::from_be_bytes(buf[DatePart::Year.get_range()].try_into().unwrap());
    let month = u8::from_le_bytes(buf[DatePart::Month.get_range()].try_into().unwrap());
    let day = u8::from_le_bytes(buf[DatePart::Day.get_range()].try_into().unwrap());
    let hours = u8::from_le_bytes(buf[DatePart::Hour.get_range()].try_into().unwrap());
    let minutes = u8::from_le_bytes(buf[DatePart::Minute.get_range()].try_into().unwrap());

    Ok(MoiDate {
        year,
        month,
        day,
        hours,
        minutes,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    println!("Download ffmpeg library...");
    unsafe {
        std::env::set_var("KEEP_ONLY_FFMPEG", "1");
    }
    if let Err(e) = auto_download() {
        eprintln!("Failed to download ffmpeg: {}", e);
        process::exit(1);
    }

    let out_path = PathBuf::from(&cli.output);

    if let Err(e) = std::fs::create_dir_all(&out_path) {
        eprintln!(
            "error: cannot create output directory {}: {}",
            cli.output, e
        );
        process::exit(1);
    }

    match &cli.input {
        Input {
            file: Some(file),
            dir: None,
        } => {
            let path = Path::new(file);
            if !path.exists() {
                eprintln!("error: file not found: {}", file);
                process::exit(1);
            }
            process_file(1usize, path, &out_path, cli.date)?;
        }
        Input {
            file: None,
            dir: Some(dir),
        } => {
            let path = Path::new(dir);
            if !path.is_dir() {
                eprintln!("error: directory not found: {}", dir);
                process::exit(1);
            }
            let entries = std::fs::read_dir(path).unwrap_or_else(|e| {
                eprintln!("error: cannot read directory {}: {}", dir, e);
                process::exit(1);
            });
            for (num, entry) in (1..).zip(entries.flatten().filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mod"))
            })) {
                process_file(num, &entry.path(), &out_path, cli.date)?;
            }
        }
        _ => unreachable!(),
    };
    Ok(())
}
