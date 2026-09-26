#[derive(Debug, Clone, PartialEq, Default)]
pub enum Action {
    Shutdown,
    ChangeTitle(String),
    OpenLink(String),
    #[default]
    Ignore,
}
