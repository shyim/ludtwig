use crate::grammar::parse_any_element;
use crate::grammar::twig::parse_any_twig;
use crate::lexer::Token;
use crate::parser::event::CompletedMarker;
use crate::parser::{Parser, RECOVERY_SET};
use crate::syntax::untyped::SyntaxKind;
use crate::T;

pub(super) fn parse_any_html(parser: &mut Parser) -> Option<CompletedMarker> {
    if parser.at(T!["<"]) {
        Some(parse_html_element(parser))
    } else if parser.at(T![word]) {
        Some(parse_html_text(parser))
    } else if parser.at(T!["<!--"]) {
        Some(parse_html_comment(parser))
    } else {
        None
    }
}

fn parse_html_text(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T![word]));
    let m = parser.start();
    parser.bump();

    while parser.at(T![word]) {
        parser.bump();
    }

    parser.complete(m, SyntaxKind::HTML_TEXT)
}

fn parse_html_comment(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T!["<!--"]));
    let m = parser.start();
    parser.bump();

    loop {
        if parser.at_end() || parser.at(T!["-->"]) {
            break;
        }

        parser.bump();
    }

    parser.expect(T!["-->"]);
    parser.complete(m, SyntaxKind::HTML_COMMENT)
}

fn parse_html_element(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T!["<"]));
    let m = parser.start();

    // parse start tag
    let starting_tag_m = parser.start();
    parser.bump();
    let tag_name = parser.expect(T![word]).map_or("", |t| t.text).to_owned();
    // parse attributes (can include twig)
    while parse_html_attribute_or_twig(parser).is_some() {}
    // parse end of starting tag
    let is_self_closing = if parser.at(T!["/>"]) {
        parser.bump();
        true
    } else {
        parser.expect(T![">"]);
        false
    };

    parser.complete(starting_tag_m, SyntaxKind::HTML_STARTING_TAG);

    // early return in case of self closing
    if is_self_closing {
        return parser.complete(m, SyntaxKind::HTML_TAG);
    }

    // parse all the children
    let body_m = parser.start();
    let mut matching_end_tag_encountered = false;
    loop {
        if parser.at(T!["</"]) {
            if let Some(Token { kind, text, .. }) = parser.at_nth_token(T![word], 1) {
                if *kind == T![word] && *text == tag_name {
                    matching_end_tag_encountered = true;
                    break; // found matching closing tag
                }
            }
        }
        if parse_any_element(parser).is_none() {
            break;
        }
    }
    parser.complete(body_m, SyntaxKind::BODY);

    // parse matching end tag if exists
    if matching_end_tag_encountered {
        // found matching closing tag
        let end_tag_m = parser.start();
        parser.expect(T!["</"]);
        parser.expect(T![word]);
        parser.expect(T![">"]);
        parser.complete(end_tag_m, SyntaxKind::HTML_ENDING_TAG);
    } else {
        // no matching end tag found!
        parser.error();
    }

    parser.complete(m, SyntaxKind::HTML_TAG)
}

fn parse_html_attribute_or_twig(parser: &mut Parser) -> Option<CompletedMarker> {
    if !parser.at(T![word]) {
        // parse any twig syntax where its children can only be html attributes (this parser)
        return parse_any_twig(parser, parse_html_attribute_or_twig);
    }

    let m = parser.start();
    parser.bump();

    if parser.at(T!["="]) {
        // attribute value
        parser.bump();
        parse_html_string_including_twig(parser);
    }

    Some(parser.complete(m, SyntaxKind::HTML_ATTRIBUTE))
}

fn parse_html_string_including_twig(parser: &mut Parser) -> CompletedMarker {
    let m = parser.start();
    parser.expect(T!["\""]);

    fn inner_str_parser(parser: &mut Parser) -> Option<CompletedMarker> {
        loop {
            if parser.at_end() || parser.at(T!("\"")) {
                break;
            }

            // TODO: needs special care for future endfor, endif, ...
            if parser.at_following(&[T!["{%"], T!["endblock"]]) {
                break;
            }

            if parse_any_twig(parser, inner_str_parser).is_none() {
                if parser.at_set(RECOVERY_SET) {
                    break;
                }

                parser.bump();
            }
        }
        None
    }

    inner_str_parser(parser);

    parser.expect(T!["\""]);
    parser.complete(m, SyntaxKind::HTML_STRING)
}

#[cfg(test)]
mod tests {
    use crate::parser::check_parse;
    use expect_test::expect;

    #[test]
    fn parse_simple_html_element() {
        check_parse(
            "<div></div>",
            expect![[r#"
                ROOT@0..11
                  HTML_TAG@0..11
                    HTML_STARTING_TAG@0..5
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_GREATER_THAN@4..5 ">"
                    BODY@5..5
                    HTML_ENDING_TAG@5..11
                      TK_LESS_THAN_SLASH@5..7 "</"
                      TK_WORD@7..10 "div"
                      TK_GREATER_THAN@10..11 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_element_with_attributes() {
        check_parse(
            "<div class=\"my-class1 my-class2\" style=\"color: blue;\"></div>",
            expect![[r#"
                ROOT@0..60
                  HTML_TAG@0..60
                    HTML_STARTING_TAG@0..54
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..33
                        TK_WORD@5..10 "class"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..33
                          TK_DOUBLE_QUOTES@11..12 "\""
                          TK_WORD@12..21 "my-class1"
                          TK_WHITESPACE@21..22 " "
                          TK_WORD@22..31 "my-class2"
                          TK_DOUBLE_QUOTES@31..32 "\""
                          TK_WHITESPACE@32..33 " "
                      HTML_ATTRIBUTE@33..53
                        TK_WORD@33..38 "style"
                        TK_EQUAL@38..39 "="
                        HTML_STRING@39..53
                          TK_DOUBLE_QUOTES@39..40 "\""
                          TK_WORD@40..46 "color:"
                          TK_WHITESPACE@46..47 " "
                          TK_WORD@47..51 "blue"
                          ERROR@51..52 ";"
                          TK_DOUBLE_QUOTES@52..53 "\""
                      TK_GREATER_THAN@53..54 ">"
                    BODY@54..54
                    HTML_ENDING_TAG@54..60
                      TK_LESS_THAN_SLASH@54..56 "</"
                      TK_WORD@56..59 "div"
                      TK_GREATER_THAN@59..60 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_element_with_children() {
        check_parse(
            "<div>hello<span>world</span>!</div>",
            expect![[r#"
                ROOT@0..35
                  HTML_TAG@0..35
                    HTML_STARTING_TAG@0..5
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_GREATER_THAN@4..5 ">"
                    BODY@5..29
                      HTML_TEXT@5..10
                        TK_WORD@5..10 "hello"
                      HTML_TAG@10..28
                        HTML_STARTING_TAG@10..16
                          TK_LESS_THAN@10..11 "<"
                          TK_WORD@11..15 "span"
                          TK_GREATER_THAN@15..16 ">"
                        BODY@16..21
                          HTML_TEXT@16..21
                            TK_WORD@16..21 "world"
                        HTML_ENDING_TAG@21..28
                          TK_LESS_THAN_SLASH@21..23 "</"
                          TK_WORD@23..27 "span"
                          TK_GREATER_THAN@27..28 ">"
                      HTML_TEXT@28..29
                        TK_WORD@28..29 "!"
                    HTML_ENDING_TAG@29..35
                      TK_LESS_THAN_SLASH@29..31 "</"
                      TK_WORD@31..34 "div"
                      TK_GREATER_THAN@34..35 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_element_with_multiple_children() {
        check_parse(
            "<div>\
                    hello<span>world</span>\
                    <p>paragraph</p>
                    <div>something</div>
                    </div>",
            expect![[r#"
                ROOT@0..112
                  HTML_TAG@0..112
                    HTML_STARTING_TAG@0..5
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_GREATER_THAN@4..5 ">"
                    BODY@5..106
                      HTML_TEXT@5..10
                        TK_WORD@5..10 "hello"
                      HTML_TAG@10..28
                        HTML_STARTING_TAG@10..16
                          TK_LESS_THAN@10..11 "<"
                          TK_WORD@11..15 "span"
                          TK_GREATER_THAN@15..16 ">"
                        BODY@16..21
                          HTML_TEXT@16..21
                            TK_WORD@16..21 "world"
                        HTML_ENDING_TAG@21..28
                          TK_LESS_THAN_SLASH@21..23 "</"
                          TK_WORD@23..27 "span"
                          TK_GREATER_THAN@27..28 ">"
                      HTML_TAG@28..65
                        HTML_STARTING_TAG@28..31
                          TK_LESS_THAN@28..29 "<"
                          TK_WORD@29..30 "p"
                          TK_GREATER_THAN@30..31 ">"
                        BODY@31..40
                          HTML_TEXT@31..40
                            TK_WORD@31..40 "paragraph"
                        HTML_ENDING_TAG@40..65
                          TK_LESS_THAN_SLASH@40..42 "</"
                          TK_WORD@42..43 "p"
                          TK_GREATER_THAN@43..44 ">"
                          TK_LINE_BREAK@44..45 "\n"
                          TK_WHITESPACE@45..65 "                    "
                      HTML_TAG@65..106
                        HTML_STARTING_TAG@65..70
                          TK_LESS_THAN@65..66 "<"
                          TK_WORD@66..69 "div"
                          TK_GREATER_THAN@69..70 ">"
                        BODY@70..79
                          HTML_TEXT@70..79
                            TK_WORD@70..79 "something"
                        HTML_ENDING_TAG@79..106
                          TK_LESS_THAN_SLASH@79..81 "</"
                          TK_WORD@81..84 "div"
                          TK_GREATER_THAN@84..85 ">"
                          TK_LINE_BREAK@85..86 "\n"
                          TK_WHITESPACE@86..106 "                    "
                    HTML_ENDING_TAG@106..112
                      TK_LESS_THAN_SLASH@106..108 "</"
                      TK_WORD@108..111 "div"
                      TK_GREATER_THAN@111..112 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_element_with_children_missing_closing_tag() {
        check_parse(
            "<div>hello<span>world!</div>",
            expect![[r#"
                ROOT@0..28
                  HTML_TAG@0..28
                    HTML_STARTING_TAG@0..5
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_GREATER_THAN@4..5 ">"
                    BODY@5..22
                      HTML_TEXT@5..10
                        TK_WORD@5..10 "hello"
                      HTML_TAG@10..22
                        HTML_STARTING_TAG@10..16
                          TK_LESS_THAN@10..11 "<"
                          TK_WORD@11..15 "span"
                          TK_GREATER_THAN@15..16 ">"
                        BODY@16..22
                          HTML_TEXT@16..22
                            TK_WORD@16..22 "world!"
                    HTML_ENDING_TAG@22..28
                      TK_LESS_THAN_SLASH@22..24 "</"
                      TK_WORD@24..27 "div"
                      TK_GREATER_THAN@27..28 ">"
                parsing consumed all tokens: true
                error at 22..22: expected word, </, word, {%, {{, {#, <, word or <!--, but found </"#]],
        );
    }

    #[test]
    fn parse_html_string_with_twig_var() {
        check_parse(
            "<div class=\"hello {{ twig }}\"></div>",
            expect![[r#"
            ROOT@0..36
              HTML_TAG@0..36
                HTML_STARTING_TAG@0..30
                  TK_LESS_THAN@0..1 "<"
                  TK_WORD@1..4 "div"
                  TK_WHITESPACE@4..5 " "
                  HTML_ATTRIBUTE@5..29
                    TK_WORD@5..10 "class"
                    TK_EQUAL@10..11 "="
                    HTML_STRING@11..29
                      TK_DOUBLE_QUOTES@11..12 "\""
                      TK_WORD@12..17 "hello"
                      TK_WHITESPACE@17..18 " "
                      TWIG_VAR@18..28
                        TK_OPEN_CURLY_CURLY@18..20 "{{"
                        TK_WHITESPACE@20..21 " "
                        TK_WORD@21..25 "twig"
                        TK_WHITESPACE@25..26 " "
                        TK_CLOSE_CURLY_CURLY@26..28 "}}"
                      TK_DOUBLE_QUOTES@28..29 "\""
                  TK_GREATER_THAN@29..30 ">"
                BODY@30..30
                HTML_ENDING_TAG@30..36
                  TK_LESS_THAN_SLASH@30..32 "</"
                  TK_WORD@32..35 "div"
                  TK_GREATER_THAN@35..36 ">"
            parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_string_with_twig_comment() {
        check_parse(
            "<div class=\"{# hello twig #}\"></div>",
            expect![[r##"
                ROOT@0..36
                  HTML_TAG@0..36
                    HTML_STARTING_TAG@0..30
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..29
                        TK_WORD@5..10 "class"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..29
                          TK_DOUBLE_QUOTES@11..12 "\""
                          TWIG_COMMENT@12..28
                            TK_OPEN_CURLY_HASHTAG@12..14 "{#"
                            TK_WHITESPACE@14..15 " "
                            TK_WORD@15..20 "hello"
                            TK_WHITESPACE@20..21 " "
                            TK_WORD@21..25 "twig"
                            TK_WHITESPACE@25..26 " "
                            TK_HASHTAG_CLOSE_CURLY@26..28 "#}"
                          TK_DOUBLE_QUOTES@28..29 "\""
                      TK_GREATER_THAN@29..30 ">"
                    BODY@30..30
                    HTML_ENDING_TAG@30..36
                      TK_LESS_THAN_SLASH@30..32 "</"
                      TK_WORD@32..35 "div"
                      TK_GREATER_THAN@35..36 ">"
                parsing consumed all tokens: true"##]],
        );
    }

    #[test]
    fn parse_html_string_with_twig_block() {
        check_parse(
            "<div class=\"hello {% block conditional %} twig {% endblock %}\"></div>",
            expect![[r#"
                ROOT@0..69
                  HTML_TAG@0..69
                    HTML_STARTING_TAG@0..63
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..62
                        TK_WORD@5..10 "class"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..62
                          TK_DOUBLE_QUOTES@11..12 "\""
                          TK_WORD@12..17 "hello"
                          TK_WHITESPACE@17..18 " "
                          TWIG_BLOCK@18..61
                            TWIG_STARTING_BLOCK@18..42
                              TK_CURLY_PERCENT@18..20 "{%"
                              TK_WHITESPACE@20..21 " "
                              TK_BLOCK@21..26 "block"
                              TK_WHITESPACE@26..27 " "
                              TK_WORD@27..38 "conditional"
                              TK_WHITESPACE@38..39 " "
                              TK_PERCENT_CURLY@39..41 "%}"
                              TK_WHITESPACE@41..42 " "
                            BODY@42..47
                              TK_WORD@42..46 "twig"
                              TK_WHITESPACE@46..47 " "
                            TWIG_ENDING_BLOCK@47..61
                              TK_CURLY_PERCENT@47..49 "{%"
                              TK_WHITESPACE@49..50 " "
                              TK_ENDBLOCK@50..58 "endblock"
                              TK_WHITESPACE@58..59 " "
                              TK_PERCENT_CURLY@59..61 "%}"
                          TK_DOUBLE_QUOTES@61..62 "\""
                      TK_GREATER_THAN@62..63 ">"
                    BODY@63..63
                    HTML_ENDING_TAG@63..69
                      TK_LESS_THAN_SLASH@63..65 "</"
                      TK_WORD@65..68 "div"
                      TK_GREATER_THAN@68..69 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_string_with_twig_block_nested() {
        check_parse(
            "<div class=\"hello {% block outer %} outer {% block inner %} inner {% endblock %}{% endblock %}\"></div>",
            expect![[r#"
                ROOT@0..102
                  HTML_TAG@0..102
                    HTML_STARTING_TAG@0..96
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..95
                        TK_WORD@5..10 "class"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..95
                          TK_DOUBLE_QUOTES@11..12 "\""
                          TK_WORD@12..17 "hello"
                          TK_WHITESPACE@17..18 " "
                          TWIG_BLOCK@18..94
                            TWIG_STARTING_BLOCK@18..36
                              TK_CURLY_PERCENT@18..20 "{%"
                              TK_WHITESPACE@20..21 " "
                              TK_BLOCK@21..26 "block"
                              TK_WHITESPACE@26..27 " "
                              TK_WORD@27..32 "outer"
                              TK_WHITESPACE@32..33 " "
                              TK_PERCENT_CURLY@33..35 "%}"
                              TK_WHITESPACE@35..36 " "
                            BODY@36..80
                              TK_WORD@36..41 "outer"
                              TK_WHITESPACE@41..42 " "
                              TWIG_BLOCK@42..80
                                TWIG_STARTING_BLOCK@42..60
                                  TK_CURLY_PERCENT@42..44 "{%"
                                  TK_WHITESPACE@44..45 " "
                                  TK_BLOCK@45..50 "block"
                                  TK_WHITESPACE@50..51 " "
                                  TK_WORD@51..56 "inner"
                                  TK_WHITESPACE@56..57 " "
                                  TK_PERCENT_CURLY@57..59 "%}"
                                  TK_WHITESPACE@59..60 " "
                                BODY@60..66
                                  TK_WORD@60..65 "inner"
                                  TK_WHITESPACE@65..66 " "
                                TWIG_ENDING_BLOCK@66..80
                                  TK_CURLY_PERCENT@66..68 "{%"
                                  TK_WHITESPACE@68..69 " "
                                  TK_ENDBLOCK@69..77 "endblock"
                                  TK_WHITESPACE@77..78 " "
                                  TK_PERCENT_CURLY@78..80 "%}"
                            TWIG_ENDING_BLOCK@80..94
                              TK_CURLY_PERCENT@80..82 "{%"
                              TK_WHITESPACE@82..83 " "
                              TK_ENDBLOCK@83..91 "endblock"
                              TK_WHITESPACE@91..92 " "
                              TK_PERCENT_CURLY@92..94 "%}"
                          TK_DOUBLE_QUOTES@94..95 "\""
                      TK_GREATER_THAN@95..96 ">"
                    BODY@96..96
                    HTML_ENDING_TAG@96..102
                      TK_LESS_THAN_SLASH@96..98 "</"
                      TK_WORD@98..101 "div"
                      TK_GREATER_THAN@101..102 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn parse_html_attribute_with_single_quotes() {
        check_parse(
            "<div claSs='my-div'>
        hello world
    </div>",
            expect![[r#"
                ROOT@0..51
                  HTML_TAG@0..51
                    HTML_STARTING_TAG@0..29
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..19
                        TK_WORD@5..10 "claSs"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..19
                          ERROR@11..12
                            TK_SINGLE_QUOTES@11..12 "'"
                          TK_WORD@12..18 "my-div"
                          TK_SINGLE_QUOTES@18..19 "'"
                      TK_GREATER_THAN@19..20 ">"
                      TK_LINE_BREAK@20..21 "\n"
                      TK_WHITESPACE@21..29 "        "
                    BODY@29..45
                      HTML_TEXT@29..45
                        TK_WORD@29..34 "hello"
                        TK_WHITESPACE@34..35 " "
                        TK_WORD@35..40 "world"
                        TK_LINE_BREAK@40..41 "\n"
                        TK_WHITESPACE@41..45 "    "
                    HTML_ENDING_TAG@45..51
                      TK_LESS_THAN_SLASH@45..47 "</"
                      TK_WORD@47..50 "div"
                      TK_GREATER_THAN@50..51 ">"
                parsing consumed all tokens: true
                error at 11..11: expected ", but found '
                error at 19..19: expected ", {%, endblock, {%, {{, {# or ", but found >"#]],
        );
    }

    #[test]
    fn parse_html_comment() {
        check_parse(
            "<!-- this is a comment --> this not <!-- but this again -->",
            expect![[r#"
                ROOT@0..59
                  HTML_COMMENT@0..27
                    TK_LESS_THAN_EXCLAMATION_MARK_MINUS_MINUS@0..4 "<!--"
                    TK_WHITESPACE@4..5 " "
                    TK_WORD@5..9 "this"
                    TK_WHITESPACE@9..10 " "
                    TK_WORD@10..12 "is"
                    TK_WHITESPACE@12..13 " "
                    TK_WORD@13..14 "a"
                    TK_WHITESPACE@14..15 " "
                    TK_WORD@15..22 "comment"
                    TK_WHITESPACE@22..23 " "
                    TK_MINUS_MINUS_GREATER_THAN@23..26 "-->"
                    TK_WHITESPACE@26..27 " "
                  HTML_TEXT@27..36
                    TK_WORD@27..31 "this"
                    TK_WHITESPACE@31..32 " "
                    TK_WORD@32..35 "not"
                    TK_WHITESPACE@35..36 " "
                  HTML_COMMENT@36..59
                    TK_LESS_THAN_EXCLAMATION_MARK_MINUS_MINUS@36..40 "<!--"
                    TK_WHITESPACE@40..41 " "
                    TK_WORD@41..44 "but"
                    TK_WHITESPACE@44..45 " "
                    TK_WORD@45..49 "this"
                    TK_WHITESPACE@49..50 " "
                    TK_WORD@50..55 "again"
                    TK_WHITESPACE@55..56 " "
                    TK_MINUS_MINUS_GREATER_THAN@56..59 "-->"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn test_html_self_closing_tag() {
        check_parse(
            "<hr/>plain<img/>text<custom/>",
            expect![[r#"
            ROOT@0..29
              HTML_TAG@0..5
                HTML_STARTING_TAG@0..5
                  TK_LESS_THAN@0..1 "<"
                  TK_WORD@1..3 "hr"
                  TK_SLASH_GREATER_THAN@3..5 "/>"
              HTML_TEXT@5..10
                TK_WORD@5..10 "plain"
              HTML_TAG@10..16
                HTML_STARTING_TAG@10..16
                  TK_LESS_THAN@10..11 "<"
                  TK_WORD@11..14 "img"
                  TK_SLASH_GREATER_THAN@14..16 "/>"
              HTML_TEXT@16..20
                TK_WORD@16..20 "text"
              HTML_TAG@20..29
                HTML_STARTING_TAG@20..29
                  TK_LESS_THAN@20..21 "<"
                  TK_WORD@21..27 "custom"
                  TK_SLASH_GREATER_THAN@27..29 "/>"
            parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn test_html_attribute_twig_var() {
        check_parse(
            "<div class=\"hello\" {{ twig }}></div>",
            expect![[r#"
                ROOT@0..36
                  HTML_TAG@0..36
                    HTML_STARTING_TAG@0..30
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      HTML_ATTRIBUTE@5..19
                        TK_WORD@5..10 "class"
                        TK_EQUAL@10..11 "="
                        HTML_STRING@11..19
                          TK_DOUBLE_QUOTES@11..12 "\""
                          TK_WORD@12..17 "hello"
                          TK_DOUBLE_QUOTES@17..18 "\""
                          TK_WHITESPACE@18..19 " "
                      TWIG_VAR@19..29
                        TK_OPEN_CURLY_CURLY@19..21 "{{"
                        TK_WHITESPACE@21..22 " "
                        TK_WORD@22..26 "twig"
                        TK_WHITESPACE@26..27 " "
                        TK_CLOSE_CURLY_CURLY@27..29 "}}"
                      TK_GREATER_THAN@29..30 ">"
                    BODY@30..30
                    HTML_ENDING_TAG@30..36
                      TK_LESS_THAN_SLASH@30..32 "</"
                      TK_WORD@32..35 "div"
                      TK_GREATER_THAN@35..36 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn test_html_attribute_twig_comment() {
        check_parse(
            "<div {# class=\"hello\" #}></div>",
            expect![[r##"
                ROOT@0..31
                  HTML_TAG@0..31
                    HTML_STARTING_TAG@0..25
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      TWIG_COMMENT@5..24
                        TK_OPEN_CURLY_HASHTAG@5..7 "{#"
                        TK_WHITESPACE@7..8 " "
                        TK_WORD@8..13 "class"
                        TK_EQUAL@13..14 "="
                        TK_DOUBLE_QUOTES@14..15 "\""
                        TK_WORD@15..20 "hello"
                        TK_DOUBLE_QUOTES@20..21 "\""
                        TK_WHITESPACE@21..22 " "
                        TK_HASHTAG_CLOSE_CURLY@22..24 "#}"
                      TK_GREATER_THAN@24..25 ">"
                    BODY@25..25
                    HTML_ENDING_TAG@25..31
                      TK_LESS_THAN_SLASH@25..27 "</"
                      TK_WORD@27..30 "div"
                      TK_GREATER_THAN@30..31 ">"
                parsing consumed all tokens: true"##]],
        );
    }

    #[test]
    fn test_html_attribute_twig_block() {
        check_parse(
            "<div {% block conditional %} class=\"hello\" {% endblock %}></div>",
            expect![[r#"
                ROOT@0..64
                  HTML_TAG@0..64
                    HTML_STARTING_TAG@0..58
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      TWIG_BLOCK@5..57
                        TWIG_STARTING_BLOCK@5..29
                          TK_CURLY_PERCENT@5..7 "{%"
                          TK_WHITESPACE@7..8 " "
                          TK_BLOCK@8..13 "block"
                          TK_WHITESPACE@13..14 " "
                          TK_WORD@14..25 "conditional"
                          TK_WHITESPACE@25..26 " "
                          TK_PERCENT_CURLY@26..28 "%}"
                          TK_WHITESPACE@28..29 " "
                        BODY@29..43
                          HTML_ATTRIBUTE@29..43
                            TK_WORD@29..34 "class"
                            TK_EQUAL@34..35 "="
                            HTML_STRING@35..43
                              TK_DOUBLE_QUOTES@35..36 "\""
                              TK_WORD@36..41 "hello"
                              TK_DOUBLE_QUOTES@41..42 "\""
                              TK_WHITESPACE@42..43 " "
                        TWIG_ENDING_BLOCK@43..57
                          TK_CURLY_PERCENT@43..45 "{%"
                          TK_WHITESPACE@45..46 " "
                          TK_ENDBLOCK@46..54 "endblock"
                          TK_WHITESPACE@54..55 " "
                          TK_PERCENT_CURLY@55..57 "%}"
                      TK_GREATER_THAN@57..58 ">"
                    BODY@58..58
                    HTML_ENDING_TAG@58..64
                      TK_LESS_THAN_SLASH@58..60 "</"
                      TK_WORD@60..63 "div"
                      TK_GREATER_THAN@63..64 ">"
                parsing consumed all tokens: true"#]],
        );
    }

    #[test]
    fn test_html_attribute_twig_block_non_attribute_body() {
        check_parse(
            "<div {% block conditional %} <hr/> {% endblock %}></div>",
            expect![[r#"
                ROOT@0..56
                  HTML_TAG@0..47
                    HTML_STARTING_TAG@0..29
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      TWIG_BLOCK@5..29
                        TWIG_STARTING_BLOCK@5..29
                          TK_CURLY_PERCENT@5..7 "{%"
                          TK_WHITESPACE@7..8 " "
                          TK_BLOCK@8..13 "block"
                          TK_WHITESPACE@13..14 " "
                          TK_WORD@14..25 "conditional"
                          TK_WHITESPACE@25..26 " "
                          TK_PERCENT_CURLY@26..28 "%}"
                          TK_WHITESPACE@28..29 " "
                        BODY@29..29
                        TWIG_ENDING_BLOCK@29..29
                    BODY@29..47
                      HTML_TAG@29..35
                        HTML_STARTING_TAG@29..35
                          TK_LESS_THAN@29..30 "<"
                          TK_WORD@30..32 "hr"
                          TK_SLASH_GREATER_THAN@32..34 "/>"
                          TK_WHITESPACE@34..35 " "
                      ERROR@35..47
                        TK_CURLY_PERCENT@35..37 "{%"
                        TK_WHITESPACE@37..38 " "
                        ERROR@38..47
                          TK_ENDBLOCK@38..46 "endblock"
                          TK_WHITESPACE@46..47 " "
                  ERROR@47..49
                    TK_PERCENT_CURLY@47..49 "%}"
                  ERROR@49..50
                    TK_GREATER_THAN@49..50 ">"
                  ERROR@50..52
                    TK_LESS_THAN_SLASH@50..52 "</"
                  HTML_TEXT@52..55
                    TK_WORD@52..55 "div"
                  ERROR@55..56
                    TK_GREATER_THAN@55..56 ">"
                parsing consumed all tokens: true
                error at 29..29: expected {%, endblock, word, {%, {{, {# or {%, but found <
                error at 29..29: expected endblock, but found <
                error at 29..29: expected %}, but found <
                error at 29..29: expected word, {%, {{, {#, /> or >, but found <
                error at 38..38: expected block, but found endblock
                error at 47..47: expected <, word or <!--, but found %}"#]],
        );
    }

    #[test]
    fn test_html_attribute_twig_block_nested() {
        check_parse(
            "<div {% block outer %} class=\"hello\" {% block inner %} style=\"color: black\" {% endblock %}{% endblock %}></div>",
            expect![[r#"
                ROOT@0..111
                  HTML_TAG@0..111
                    HTML_STARTING_TAG@0..105
                      TK_LESS_THAN@0..1 "<"
                      TK_WORD@1..4 "div"
                      TK_WHITESPACE@4..5 " "
                      TWIG_BLOCK@5..104
                        TWIG_STARTING_BLOCK@5..23
                          TK_CURLY_PERCENT@5..7 "{%"
                          TK_WHITESPACE@7..8 " "
                          TK_BLOCK@8..13 "block"
                          TK_WHITESPACE@13..14 " "
                          TK_WORD@14..19 "outer"
                          TK_WHITESPACE@19..20 " "
                          TK_PERCENT_CURLY@20..22 "%}"
                          TK_WHITESPACE@22..23 " "
                        BODY@23..90
                          HTML_ATTRIBUTE@23..37
                            TK_WORD@23..28 "class"
                            TK_EQUAL@28..29 "="
                            HTML_STRING@29..37
                              TK_DOUBLE_QUOTES@29..30 "\""
                              TK_WORD@30..35 "hello"
                              TK_DOUBLE_QUOTES@35..36 "\""
                              TK_WHITESPACE@36..37 " "
                          TWIG_BLOCK@37..90
                            TWIG_STARTING_BLOCK@37..55
                              TK_CURLY_PERCENT@37..39 "{%"
                              TK_WHITESPACE@39..40 " "
                              TK_BLOCK@40..45 "block"
                              TK_WHITESPACE@45..46 " "
                              TK_WORD@46..51 "inner"
                              TK_WHITESPACE@51..52 " "
                              TK_PERCENT_CURLY@52..54 "%}"
                              TK_WHITESPACE@54..55 " "
                            BODY@55..76
                              HTML_ATTRIBUTE@55..76
                                TK_WORD@55..60 "style"
                                TK_EQUAL@60..61 "="
                                HTML_STRING@61..76
                                  TK_DOUBLE_QUOTES@61..62 "\""
                                  TK_WORD@62..68 "color:"
                                  TK_WHITESPACE@68..69 " "
                                  TK_WORD@69..74 "black"
                                  TK_DOUBLE_QUOTES@74..75 "\""
                                  TK_WHITESPACE@75..76 " "
                            TWIG_ENDING_BLOCK@76..90
                              TK_CURLY_PERCENT@76..78 "{%"
                              TK_WHITESPACE@78..79 " "
                              TK_ENDBLOCK@79..87 "endblock"
                              TK_WHITESPACE@87..88 " "
                              TK_PERCENT_CURLY@88..90 "%}"
                        TWIG_ENDING_BLOCK@90..104
                          TK_CURLY_PERCENT@90..92 "{%"
                          TK_WHITESPACE@92..93 " "
                          TK_ENDBLOCK@93..101 "endblock"
                          TK_WHITESPACE@101..102 " "
                          TK_PERCENT_CURLY@102..104 "%}"
                      TK_GREATER_THAN@104..105 ">"
                    BODY@105..105
                    HTML_ENDING_TAG@105..111
                      TK_LESS_THAN_SLASH@105..107 "</"
                      TK_WORD@107..110 "div"
                      TK_GREATER_THAN@110..111 ">"
                parsing consumed all tokens: true"#]],
        );
    }
}
