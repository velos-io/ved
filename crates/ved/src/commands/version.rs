use crate::error::Result;
use crate::output::Outputter;

pub(crate) fn handle_version(output: &dyn Outputter) -> Result<()> {
    output.println(&format!(
        "ved {} (built {})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_TIMESTAMP")
    ));
    Ok(())
}
