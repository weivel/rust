//! A value tree written as JSON: the escapes of a text, a record's member order, nesting, and the numbers JSON has no
//! form for.

use weivel_rust::host::{NonFiniteNumber, Tree};

fn text(value: &str) -> Tree {
    Tree::Text(value.to_owned())
}

#[test]
fn a_text_escapes_what_json_cannot_hold_raw() {
    assert_eq!(
        text("say \"hi\"\\ \n\r\t\u{1}\u{1f} é").to_json(),
        Ok(r#""say \"hi\"\\ \n\r\t\u0001\u001f é""#.to_owned())
    );
}

#[test]
fn a_record_keeps_its_members_in_the_order_it_holds_them() {
    let tree = Tree::Record(vec![
        ("b".to_owned(), Tree::Number(1.0)),
        ("a".to_owned(), Tree::Boolean(false)),
        ("c\"".to_owned(), Tree::Null),
    ]);
    assert_eq!(
        tree.to_json(),
        Ok(r#"{"b":1,"a":false,"c\"":null}"#.to_owned())
    );
}

#[test]
fn nested_lists_and_records_are_written_compactly() {
    let tree = Tree::List(vec![
        Tree::Record(vec![(
            "inner".to_owned(),
            Tree::List(vec![Tree::Number(-0.5), Tree::Number(1e300), text("x")]),
        )]),
        Tree::List(Vec::new()),
        Tree::Record(Vec::new()),
        Tree::Boolean(true),
    ]);
    let written = tree.to_json().expect("every number is finite");
    assert_eq!(
        written,
        format!(r#"[{{"inner":[-0.5,{},"x"]}},[],{{}},true]"#, 1e300)
    );
}

#[test]
fn a_number_json_has_no_form_for_is_refused_wherever_it_sits() {
    for refused in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let tree = Tree::Record(vec![(
            "deep".to_owned(),
            Tree::List(vec![Tree::Null, Tree::Number(refused)]),
        )]);
        let error = tree
            .to_json()
            .expect_err("a non-finite number has no JSON form");
        assert_eq!(error.0.to_bits(), refused.to_bits());
        assert!(error.to_string().contains("no JSON form"), "{error}");
    }
    assert_eq!(
        Tree::Number(f64::INFINITY).to_json(),
        Err(NonFiniteNumber(f64::INFINITY))
    );
}
