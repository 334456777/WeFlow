//! Synthetic workbook probe; no WeChat data is opened.
//! Run with an explicitly writable TEMP/TMP to separate environment from content.
use rust_xlsxwriter::Workbook;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let output = args.next().expect("xlsx_probe <output.xlsx> [rows]");
    let rows: u32 = args.next().map_or(Ok(20_001), |s| s.parse())?;
    let temp = std::env::temp_dir();
    println!(
        "temp directory exists: {}; equals TMP: {}",
        temp.is_dir(),
        std::env::var_os("TMP").is_some_and(|v| temp == std::path::PathBuf::from(v))
    );
    let marker = temp.join(format!("weflow-xlsx-probe-{}.tmp", std::process::id()));
    let ordinary = std::fs::write(&marker, b"synthetic");
    println!("ordinary temporary write: {ordinary:?}");
    if ordinary.is_ok() {
        std::fs::remove_file(&marker)?;
    }
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet_with_constant_memory();
    for row in 0..rows {
        sheet.write_number(row, 0, row as f64)?;
        sheet.write_string(row, 1, "synthetic fixture")?;
    }
    workbook.save(output)?;
    println!("synthetic workbook saved: {rows} rows");
    Ok(())
}
