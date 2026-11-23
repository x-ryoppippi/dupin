use super::catalog::Script;
use dupin_core::script::{ScriptRun, split_args};
use rune::TextEdit;

pub struct Entry {
    pub key: String,
    pub script: Script,
    pub query: TextEdit,
}

impl Entry {
    pub fn command(&self) -> Result<ScriptRun, String> {
        let args = split_args(&self.query.text)?;
        self.script.header.bind_args(&args)?;
        Ok(ScriptRun {
            path: self.script.path.clone(),
            title: self.script.header.title.clone(),
            args,
            mode: self.script.header.mode,
        })
    }

    pub fn is_current(&self, session: &dupin_core::Session, catalog: &super::catalog::Catalog) -> bool {
        catalog.scripts.get(&self.script.path) == Some(&self.script)
            && session.items().iter().enumerate().any(|(i, item)| {
                session.result_key(i) == self.key
                    && item.action(super::provider::ARGUMENTS)
                        == Some(&dupin_core::Action::ScriptArguments(self.script.path.clone()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_previews_literal_argv_and_refuses_incomplete_arguments() {
        let (header, _) = dupin_core::script::Header::parse(b"# @dupin.argument values+", "Fixture").unwrap();
        let mut entry = Entry {
            key: "fixture".into(),
            script: Script {
                path: "/fixture/script".into(),
                header,
            },
            query: TextEdit::new(""),
        };
        assert!(entry.command().is_err());
        entry.query = TextEdit::new("'hello world' $HOME ~ *.rs");
        assert_eq!(
            entry.command().unwrap().args,
            ["hello world", "$HOME", "~", "*.rs"]
        );
        entry.query = TextEdit::new("'unfinished");
        assert!(entry.command().is_err());
    }
}
