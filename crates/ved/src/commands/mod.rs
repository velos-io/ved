use crate::cli::Commands;
use crate::context::WorkspaceContext;
use crate::error::Result;

pub(crate) mod build;
pub(crate) mod compile;
pub(crate) mod config;
pub(crate) mod deinit;
pub(crate) mod down;
pub(crate) mod init;
pub(crate) mod ps;
pub(crate) mod reset;
pub(crate) mod sync;
pub(crate) mod test;
pub(crate) mod up;
pub(crate) mod validate;
pub(crate) mod version;

pub(crate) fn dispatch_with_output(
    command: Commands,
    ctx: &WorkspaceContext,
    output: &dyn crate::output::Outputter,
) -> Result<()> {
    match command {
        Commands::Compile(args) => compile::handle_compile_with_output(ctx, args, output),
        Commands::Up(args) => {
            up::handle_up_with_output(&crate::orchestrator::HostDeployer, ctx, args, output)
        }
        Commands::Down(args) => {
            down::handle_down_with_output(ctx, args, &crate::orchestrator::HostDeployer, output)
        }
        Commands::Ps(args) => {
            ps::handle_ps_with_output(&crate::orchestrator::HostDeployer, ctx, args, output)
        }
        Commands::Reset(args) => {
            reset::handle_reset_with_output(ctx, args, &crate::orchestrator::HostDeployer, output)
        }
        Commands::Test(args) => {
            test::handle_test_with_output(ctx, args, &crate::orchestrator::HostDeployer, output)
        }
        Commands::Validate(args) => validate::handle_validate(ctx, args, output),
        Commands::Init(args) => init::handle_init(ctx, args, output),
        Commands::Deinit(args) => deinit::handle_deinit(ctx, args, output),
        Commands::Config(args) => config::handle_config(ctx, args, output),
        Commands::Sync(args) => sync::handle_sync(ctx, args, output),
        Commands::Build(args) => build::handle_build(ctx, args, output),
        Commands::Version => version::handle_version(output),
    }
}
