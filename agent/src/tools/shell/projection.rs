use super::*;

pub(super) fn text(result: &ShellResult) -> String {
    future_rpc::shell_result::model_summary(result)
}
