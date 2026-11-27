use rune::{Color, El, TextStyle, col, row, text, text_input};

use crate::ui::Ui;

use super::header::HEADER_H;
use super::results::{list_order, visible_rows};

pub(super) fn view(s: &Ui, values: &[serde_json::Value], selected: usize) -> El {
    let field = text_input(&s.query, TextStyle::new(22.0, 400.0))
        .key("pick-query")
        .single_line()
        .color(Color::WHITE)
        .on_edit(|s: &mut Ui| &mut s.query)
        .on_input_key(Ui::key)
        .on_input_change(Ui::edited);
    picker_view(
        field,
        values,
        &s.query.text,
        selected,
        visible_rows(s.list_budget, s.theme.row_height as f32),
        s.theme.row_height as f32,
        s.grows_up(),
    )
    .on_key(Ui::key)
}

fn picker_view(
    field: El,
    values: &[serde_json::Value],
    query: &str,
    selected: usize,
    shown: usize,
    row_height: f32,
    grows_up: bool,
) -> El {
    let mut parts = vec![
        row()
            .w_full()
            .h(HEADER_H)
            .px(16.0)
            .center_y()
            .child(field.grow(1.0)),
    ];
    for (index, original) in crate::ui::pick_indices(values, query)
        .into_iter()
        .enumerate()
        .skip(selected.saturating_sub(shown - 1))
        .take(shown)
    {
        let value = &values[original];
        let label = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        parts.push(
            row()
                .key(format!("pick-item-{original}"))
                .w_full()
                .h(row_height)
                .px(16.0)
                .center_y()
                .fill(if index == selected {
                    Color {
                        r: 0.2,
                        g: 0.27,
                        b: 0.4,
                        a: 1.0,
                    }
                } else {
                    Color::BLACK
                })
                .child(text(label).color(Color::WHITE)),
        );
    }
    col()
        .key("picker")
        .w_full()
        .fill(Color::BLACK)
        .children(list_order(parts, grows_up))
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    #[test]
    #[ignore = "offscreen picker layout, run explicitly"]
    fn picker_rows_have_space_for_every_visible_value() {
        struct Preview {
            values: Vec<serde_json::Value>,
            edit: rune::TextEdit,
        }
        let app = rune::App::new(rune::AppSpec::new(
            "Picker layout",
            (680.0, 168.0),
            || Preview {
                values: vec![serde_json::json!("red"), serde_json::json!("blue")],
                edit: rune::TextEdit::new(""),
            },
            |s: &Preview, _| {
                picker_view(
                    text_input(&s.edit, TextStyle::new(22.0, 400.0)),
                    &s.values,
                    "",
                    0,
                    8,
                    52.0,
                    false,
                )
            },
        ));
        let root = std::env::temp_dir().join("dupin165-picker-layout");
        let tree = root.with_extension("json");
        let image = root.with_extension("png");
        rune::tooling::run_windows_with_args(
            Default::default(),
            [
                "render",
                "--tree",
                tree.to_str().unwrap(),
                "--snap",
                image.to_str().unwrap(),
                "--size",
                "680x168",
            ]
            .map(str::to_owned),
            move |_| [rune::WindowSpec::from_app("picker", app)],
        );
        let tree: serde_json::Value = serde_json::from_slice(&std::fs::read(tree).unwrap()).unwrap();
        let elements = tree["elements"].as_array().unwrap();
        for (index, y) in [(0, 64.0), (1, 116.0)] {
            let row = elements
                .iter()
                .find(|element| element["key"] == format!("pick-item-{index}"))
                .unwrap();
            assert_eq!(row["rect"][1].as_f64().unwrap(), y);
            assert_eq!(row["rect"][3].as_f64().unwrap(), 52.0);
            assert!(row["rect"][1].as_f64().unwrap() + row["rect"][3].as_f64().unwrap() <= 168.0);
        }
        println!("{}", image.display());
    }
}
