use super::*;

const HEADER: &str = concat!(
    "ISO-10303-21;HEADER;",
    "FILE_DESCRIPTION(('Test'),'2;1');",
    "FILE_NAME('x','1970-01-01T00:00:00',(''),(''),'','','');",
    "FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));"
);
const END: &str = "ENDSEC;END-ISO-10303-21;";

fn document(data: &str) -> String {
    format!("{HEADER} ENDSEC; DATA; {data} {END}")
}

#[test]
fn multiline_entities_and_complex_components_preserve_borrowed_spelling() {
    let text = format!(
        "\n/* start */ {HEADER} SECTION_LANGUAGE('','en'); ENDSEC; DATA /* gap */;\n\
         #1=EXAMPLE(\n'can''t? It''s naïve /* literal */',(#2,+1.E-7,-3.5),.T.,$,*,LENGTH_MEASURE(2.));\n\
         #2=(LENGTH_UNIT() NAMED_UNIT(*) /* between components */ SI_UNIT(.MILLI.,.METRE.));\n\
         {END} /* trailing comment */ \n"
    );
    let records = parse(&text).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[&1],
        [Entity {
            name: "EXAMPLE",
            args: vec![
                Value::String("can''t? It''s naïve /* literal */"),
                Value::List(vec![
                    Value::Reference(2),
                    Value::Number(1e-7),
                    Value::Number(-3.5)
                ]),
                Value::Enum("T"),
                Value::Omitted,
                Value::Derived,
                Value::Typed(Entity {
                    name: "LENGTH_MEASURE",
                    args: vec![Value::Number(2.)],
                }),
            ],
        }]
    );
    assert_eq!(
        records[&2],
        [
            Entity {
                name: "LENGTH_UNIT",
                args: vec![]
            },
            Entity {
                name: "NAMED_UNIT",
                args: vec![Value::Derived]
            },
            Entity {
                name: "SI_UNIT",
                args: vec![Value::Enum("MILLI"), Value::Enum("METRE")]
            },
        ]
    );
}

#[test]
fn corrupt_or_ambiguous_record_syntax_is_rejected() {
    for invalid in [
        "#1=EXAMPLE();#1=EXAMPLE();", // Duplicate instance ID.
        "#0=EXAMPLE();",
        "# 1=EXAMPLE();",
        "#1=EXAMPLE(#0);",
        "#1=EXAMPLE(# 2);",
        "#1=EXAMPLE('unclosed);",
        "#1=EXAMPLE(.MILLI);",
        "#1=EXAMPLE(1.E+);",
        "#1=EXAMPLE(1.E+999);",
        "#1=EXAMPLE(1. 2.);",
        "#1=EXAMPLE(1.,);",
        "#1=EXAMPLE(€);",
        "#1=();",
        "#1=(A() A());",
        "#1=example();",
        "#1=EXAMPLE()",
        "#1=EXAMPLE(); /* unterminated",
    ] {
        assert_eq!(
            parse(&document(invalid)),
            Err(ReadError::Invalid),
            "{invalid}"
        );
    }
    for text in [
        "not STEP".to_owned(),
        "ISO-10303-21;HEADER;FILE_NAME();".to_owned(),
        format!("{HEADER} FILE_NAME(); ENDSEC; DATA; {END}"),
        format!("{HEADER} ENDSEC; NOT_DATA; {END}"),
        format!("{} appended", document("")),
    ] {
        assert_eq!(parse(&text), Err(ReadError::Invalid), "{text}");
    }
}

#[test]
fn unsupported_extensions_are_separate_from_corruption() {
    for text in [
        document("#1=EXAMPLE(\"0A\");"),
        format!("{HEADER} ENDSEC; DATA /* comment */('name',('AUTOMOTIVE_DESIGN')); {END}"),
        format!("{HEADER} ENDSEC; DATA; ENDSEC; DATA; {END}"),
    ] {
        assert_eq!(parse(&text), Err(ReadError::Unsupported));
    }
}

#[test]
fn large_or_deep_inputs_stop_at_the_parser_budget() {
    eprintln!(
        "STEP parser storage per value {} bytes, per entity {} bytes; limits {MAX_BYTES} bytes, {MAX_ENTITIES} entities, {MAX_VALUES} values",
        std::mem::size_of::<Value<'_>>(),
        std::mem::size_of::<Entity<'_>>(),
    );
    assert_eq!(parse(&" ".repeat(MAX_BYTES + 1)), Err(ReadError::Limit));
    assert_eq!(parse(&document("#4294967296=A();")), Err(ReadError::Limit));
    assert_eq!(
        parse(&document(&format!(
            "#1={}();",
            "A".repeat(MAX_TOKEN_BYTES + 1)
        ))),
        Err(ReadError::Limit)
    );
    let long_digits = "1".repeat(MAX_TOKEN_BYTES + 1);
    let long_enum = "A".repeat(MAX_TOKEN_BYTES + 1);
    for data in [
        format!("#{long_digits}=A();"),
        format!("#1=A({long_digits});"),
        format!("#1=A(.{long_enum}.);"),
    ] {
        assert_eq!(parse(&document(&data)), Err(ReadError::Limit));
    }
    assert_eq!(
        parse(&document(&format!(
            "#1=A('{}');",
            "x".repeat(MAX_TOKEN_BYTES + 1)
        ))),
        Err(ReadError::Limit)
    );
    assert_eq!(
        parse(&document(&format!(
            "#1=A('{}''');",
            "x".repeat(MAX_TOKEN_BYTES)
        ))),
        Err(ReadError::Limit)
    );
    assert_eq!(
        parse(&document(&format!(
            "#1=A({}0{});",
            "(".repeat(MAX_DEPTH),
            ")".repeat(MAX_DEPTH)
        ))),
        Err(ReadError::Limit)
    );
    let components = (0..=MAX_COMPONENTS)
        .map(|i| format!("A{i}()"))
        .collect::<String>();
    assert_eq!(
        parse(&document(&format!("#1=({components});"))),
        Err(ReadError::Limit)
    );

    let mut many = String::new();
    use std::fmt::Write;
    for id in 1..=MAX_ENTITIES + 1 {
        write!(many, "#{id}=A();").unwrap();
    }
    assert_eq!(parse(&document(&many)), Err(ReadError::Limit));
    let args = "0,".repeat(MAX_VALUES);
    assert_eq!(
        parse(&document(&format!("#1=A({args}0);"))),
        Err(ReadError::Limit)
    );
}
