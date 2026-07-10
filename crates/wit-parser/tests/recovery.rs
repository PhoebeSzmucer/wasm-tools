use wit_parser::{ParseErrorKind, SourceMap, UnresolvedPackageGroup};

fn map_of(files: &[(&str, &str)]) -> SourceMap {
    let mut map = SourceMap::new();
    for &(path, contents) in files {
        map.push_str(path, contents);
    }
    map
}

fn interface_names(group: &UnresolvedPackageGroup) -> Vec<&str> {
    group
        .main
        .interfaces
        .iter()
        .filter_map(|(_, iface)| iface.name.as_deref())
        .collect()
}

#[test]
fn recovery_all_valid() {
    let mut source_map = SourceMap::new();
    source_map.push_str(
        "file1.wit",
        r#"
        package wasi-example:cli;
        interface run1 {
            run1: async func() -> result;
        }
    "#,
    );
    source_map.push_str(
        "file2.wit",
        r#"
        package wasi-example:cli;
        interface run2 {
            run2: async func() -> result;
        }
    "#,
    );
    let (result, errors) = source_map.parse_recovering();
    let group = result.unwrap();

    assert!(errors.is_empty());
    let iface_names: Vec<Option<String>> = group
        .main
        .interfaces
        .into_iter()
        .map(|(_, iface)| iface.name)
        .collect();
    assert_eq!(iface_names, vec![Some("run1".into()), Some("run2".into())]);
}

#[test]
fn recovery_one_file_invalid() {
    let mut source_map = SourceMap::new();
    source_map.push_str(
        "file1.wit",
        r#"
        invalid
    "#,
    );
    source_map.push_str(
        "file2.wit",
        r#"
        package wasi-example:cli;
        interface run2 {
            run2: async func() -> result;
        }
    "#,
    );
    let (result, errors) = source_map.parse_recovering();
    let group = result.unwrap();

    let [error] = &errors[..] else {
        panic!("Expected exactly 1 error");
    };

    match error.kind() {
        ParseErrorKind::Syntax { message, .. } => {
            assert!(
                message.contains("expected"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected a syntax error, got: {other:#?}"),
    }

    let rendered = error.render(&group.source_map);
    assert!(
        rendered.contains("file1.wit"),
        "error does not point into file1:\n{rendered}"
    );

    let iface_names: Vec<Option<String>> = group
        .main
        .interfaces
        .into_iter()
        .map(|(_, iface)| iface.name)
        .collect();
    assert_eq!(iface_names, vec![Some("run2".into())]);
}

#[test]
fn recovery_package_name_mismatch() {
    let map = map_of(&[
        (
            "a.wit",
            "package foo:one;\ninterface iface-a { f: func(); }",
        ),
        (
            "b.wit",
            "package foo:two;\ninterface iface-b { f: func(); }",
        ),
        (
            "c.wit",
            "package foo:one;\ninterface iface-c { f: func(); }",
        ),
    ]);
    let (result, errors) = map.parse_recovering();
    let group = result.unwrap();

    let [error] = &errors[..] else {
        panic!("expected exactly 1 error, got: {errors:#?}");
    };
    match error.kind() {
        ParseErrorKind::Syntax { message, .. } => {
            assert!(
                message.contains("does not match"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected a syntax error, got: {other:#?}"),
    }
    let rendered = error.render(&group.source_map);
    assert!(
        rendered.contains("b.wit"),
        "error should point into b.wit:\n{rendered}"
    );

    assert_eq!(group.main.name.to_string(), "foo:one");
    // The mismatched file's contents are dropped, but files pushed *after*
    // the failed push must still land: this is the assertion that fails if
    // `Resolver::push` ever stops leaving the resolver usable on error.
    assert_eq!(interface_names(&group), ["iface-a", "iface-c"]);
}

#[test]
fn recovery_broken_nested_package() {
    let map = map_of(&[(
        "main.wit",
        r#"
        package foo:main;
        interface good { g: func(); }
        package foo:broken {
            interface dup {}
            interface dup {}
        }
        package foo:ok {
            interface fine { f: func(); }
        }
        "#,
    )]);
    let (result, errors) = map.parse_recovering();
    let group = result.unwrap();

    let [error] = &errors[..] else {
        panic!("expected exactly 1 error, got: {errors:#?}");
    };
    let rendered = error.render(&group.source_map);
    assert!(
        rendered.contains("dup"),
        "error should point at the duplicate interface:\n{rendered}"
    );

    // Only the broken nested package is discarded: the main package's
    // items and the healthy nested package both survive.
    assert_eq!(interface_names(&group), ["good"]);
    let nested_names: Vec<String> = group.nested.iter().map(|p| p.name.to_string()).collect();
    assert_eq!(nested_names, ["foo:ok"]);
}

#[test]
fn recovery_total_failure_returns_map_and_all_errors() {
    let map = map_of(&[("a.wit", "definitely not wit"), ("b.wit", "also not wit")]);
    let (result, errors) = map.parse_recovering();
    let map = match result {
        Err(map) => map,
        Ok(group) => panic!("expected total failure, got: {:?}", group.main.name),
    };

    // One parse error per file, plus the terminal "no `package` header" error.
    assert!(!errors.is_empty(), "Err result must come with errors");
    let rendered: Vec<String> = errors.iter().map(|e| e.render(&map)).collect();
    assert_eq!(rendered.len(), 3, "got: {rendered:#?}");
    assert!(rendered[0].contains("a.wit"), "{}", rendered[0]);
    assert!(rendered[1].contains("b.wit"), "{}", rendered[1]);
    assert!(rendered[2].contains("package"), "{}", rendered[2]);
}

#[test]
fn recovery_first_error_matches_parse() {
    let broken_inputs: &[&[(&str, &str)]] = &[
        &[("a.wit", "invalid")],
        &[("a.wit", "invalid"), ("b.wit", "package foo:ok;")],
        &[("a.wit", "package foo:one;"), ("b.wit", "package foo:two;")],
        &[(
            "main.wit",
            "package foo:main;\npackage foo:broken { interface dup {} interface dup {} }",
        )],
    ];
    for files in broken_inputs {
        let map = map_of(files);
        let (_, errors) = map.clone().parse_recovering();
        let (_, first) = map.parse().unwrap_err();
        assert_eq!(errors.first(), Some(&first), "input: {files:?}");
    }
}

#[test]
fn recovery_multiple_broken_files_in_path_order() {
    // Deliberately pushed out of order: errors must follow path order, not
    // push order.
    let map = map_of(&[
        ("c.wit", "broken here"),
        ("a.wit", "broken too"),
        ("b.wit", "package foo:ok;\ninterface ok { f: func(); }"),
    ]);
    let (result, errors) = map.parse_recovering();
    let group = result.unwrap();

    let [first, second] = &errors[..] else {
        panic!("expected exactly 2 errors, got: {errors:#?}");
    };
    let first = first.render(&group.source_map);
    let second = second.render(&group.source_map);
    assert!(first.contains("a.wit"), "{first}");
    assert!(second.contains("c.wit"), "{second}");
    assert_eq!(interface_names(&group), ["ok"]);
}
