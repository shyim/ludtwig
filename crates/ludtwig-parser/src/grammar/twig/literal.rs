use crate::grammar::parse_many;
use crate::grammar::twig::expression::parse_twig_expression;
use crate::parser::event::CompletedMarker;
use crate::parser::{ParseErrorBuilder, Parser};
use crate::syntax::untyped::SyntaxKind;
use crate::T;
use once_cell::sync::Lazy;
use regex::Regex;

// TODO: maybe allow more here to partly support twig.js. Needs testing on real world templates
static TWIG_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*$"#).unwrap());

pub(crate) fn parse_twig_literal(parser: &mut Parser) -> Option<CompletedMarker> {
    let last_node = if parser.at(T![number]) {
        Some(parse_twig_number(parser))
    } else if parser.at_set(&[T!["\""], T!["'"]]) {
        Some(parse_twig_string(parser, true))
    } else if parser.at(T!["["]) {
        Some(parse_twig_array(parser))
    } else if parser.at(T!["null"]) {
        Some(parse_twig_null(parser))
    } else if parser.at_set(&[T!["true"], T!["false"]]) {
        Some(parse_twig_boolean(parser))
    } else if parser.at(T!["{"]) {
        Some(parse_twig_hash(parser))
    } else {
        parse_twig_name_postfix(parser)
    };

    let mut node = match last_node {
        None => return None,
        Some(last_node) => last_node,
    };

    // parse any amount of filters
    parse_many(
        parser,
        |_| false,
        |p| {
            if p.at(T!["|"]) {
                node = parse_twig_filter(p, node.clone());
            }
        },
    );

    Some(node)
}

fn parse_twig_number(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T![number]));
    let m = parser.start();
    parser.bump();

    parser.complete(m, SyntaxKind::TWIG_LITERAL_NUMBER)
}

pub(crate) fn parse_twig_string(
    parser: &mut Parser,
    mut interpolation_allowed: bool,
) -> CompletedMarker {
    debug_assert!(parser.at_set(&[T!["\""], T!["'"]]));
    let m = parser.start();
    let starting_quote_token = parser.bump();
    let quote_kind = starting_quote_token.kind;
    interpolation_allowed = match quote_kind {
        T!["\""] => interpolation_allowed,
        _ => false, // interpolation only allowed in double quoted strings,
    };

    let m_inner = parser.start();
    parse_many(
        parser,
        |p| p.at(quote_kind),
        |p| {
            if p.at_following(&[T!["\\"], quote_kind]) {
                // escaped quote should be consumed
                p.bump();
                p.bump();
            } else if p.at(T!["#{"]) {
                if !interpolation_allowed {
                    let opening_token = p.bump();
                    let interpolation_error = ParseErrorBuilder::new(
                        "no string interpolation, because it isn't allowed here",
                    )
                    .at_token(opening_token);
                    p.add_error(interpolation_error);
                    return;
                }

                // found twig expression in string (string interpolation)
                p.explicitly_consume_trivia(); // keep trivia out of interpolation node (its part of the raw string)
                let interpolation_m = p.start();
                p.bump();
                if parse_twig_expression(p).is_none() {
                    p.add_error(ParseErrorBuilder::new("twig expression"));
                }
                p.expect(T!["}"]);
                p.complete(
                    interpolation_m,
                    SyntaxKind::TWIG_LITERAL_STRING_INTERPOLATION,
                );
            } else {
                // bump the token inside the string
                p.bump();
            }
        },
    );
    parser.explicitly_consume_trivia(); // consume any trailing trivia inside the string
    parser.complete(m_inner, SyntaxKind::TWIG_LITERAL_STRING_INNER);

    parser.expect(quote_kind);
    parser.complete(m, SyntaxKind::TWIG_LITERAL_STRING)
}

fn parse_twig_array(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T!["["]));
    let m = parser.start();
    parser.bump();

    parse_many(
        parser,
        |p| p.at(T!["]"]),
        |p| {
            parse_twig_expression(p);

            if p.at(T![","]) {
                // consume separator
                p.bump();
            }
        },
    );

    parser.expect(T!["]"]);
    parser.complete(m, SyntaxKind::TWIG_LITERAL_ARRAY)
}

fn parse_twig_null(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T!["null"]));
    let m = parser.start();
    parser.bump();

    parser.complete(m, SyntaxKind::TWIG_LITERAL_NULL)
}

fn parse_twig_boolean(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at_set(&[T!["true"], T!["false"]]));
    let m = parser.start();
    parser.bump();

    parser.complete(m, SyntaxKind::TWIG_LITERAL_BOOLEAN)
}

fn parse_twig_hash(parser: &mut Parser) -> CompletedMarker {
    debug_assert!(parser.at(T!["{"]));
    let m = parser.start();
    parser.bump();

    parse_many(
        parser,
        |p| p.at(T!["}"]),
        |p| {
            parse_twig_hash_pair(p);

            if p.at(T![","]) {
                // consume separator
                p.bump();
            }
        },
    );

    parser.expect(T!["}"]);
    parser.complete(m, SyntaxKind::TWIG_LITERAL_HASH)
}

fn parse_twig_hash_pair(parser: &mut Parser) -> Option<CompletedMarker> {
    let key = if parser.at(T![number]) {
        let m = parse_twig_number(parser);
        let preceded = parser.precede(m);
        parser.complete(preceded, SyntaxKind::TWIG_LITERAL_HASH_KEY)
    } else if parser.at_set(&[T!["'"], T!["\""]]) {
        let m = parse_twig_string(parser, false); // no interpolation in keys
        let preceded = parser.precede(m);
        parser.complete(preceded, SyntaxKind::TWIG_LITERAL_HASH_KEY)
    } else if parser.at(T!["("]) {
        let m = parser.start();
        parser.bump();
        if parse_twig_expression(parser).is_none() {
            parser.add_error(ParseErrorBuilder::new("twig expression"))
        }
        parser.expect(T![")"]);
        parser.complete(m, SyntaxKind::TWIG_LITERAL_HASH_KEY)
    } else {
        let token_text = parser.peek_token()?.text;
        if TWIG_NAME_REGEX.is_match(token_text) {
            let m = parser.start();
            parser.bump_as(SyntaxKind::TK_WORD);
            parser.complete(m, SyntaxKind::TWIG_LITERAL_HASH_KEY)
        } else {
            return None;
        }
    };

    // check if key exists
    if parser.at(T![":"]) {
        parser.bump();
        if parse_twig_expression(parser).is_none() {
            parser.add_error(ParseErrorBuilder::new("value as twig expression"))
        }
    }

    let preceded = parser.precede(key);
    Some(parser.complete(preceded, SyntaxKind::TWIG_LITERAL_HASH_PAIR))
}

fn parse_twig_name_postfix(parser: &mut Parser) -> Option<CompletedMarker> {
    let mut node = parse_twig_name(parser)?;

    parse_many(
        parser,
        |_| false,
        |p| {
            if p.at(T!["."]) {
                node = parse_twig_accessor(p, node.clone());
            } else if p.at(T!["["]) {
                node = parse_twig_indexer(p, node.clone());
            } else if p.at(T!["("]) {
                node = parse_twig_function(p, node.clone());
            }
        },
    );

    Some(node)
}

pub(crate) fn parse_twig_filter(
    parser: &mut Parser,
    mut last_node: CompletedMarker,
) -> CompletedMarker {
    debug_assert!(parser.at(T!["|"]));

    // wrap last_node in an operand and create outer marker
    let m = parser.precede(last_node);
    last_node = parser.complete(m, SyntaxKind::TWIG_OPERAND);
    let outer = parser.precede(last_node);

    // bump the operator
    parser.bump();

    // parse the rhs and wrap it also in an operand
    let m = parser.start();
    if parse_twig_name(parser).is_none() {
        parser.add_error(ParseErrorBuilder::new("twig filter"));
    } else if parser.at(T!["("]) {
        parser.bump();
        // parse any amount of arguments
        let arguments_m = parser.start();
        parse_many(
            parser,
            |p| p.at(T![")"]),
            |p| {
                parse_twig_function_argument(p);
                if p.at(T![","]) {
                    p.bump();
                }
            },
        );
        parser.complete(arguments_m, SyntaxKind::TWIG_ARGUMENTS);
        parser.expect(T![")"]);
    }
    parser.complete(m, SyntaxKind::TWIG_OPERAND);

    // complete the outer marker
    parser.complete(outer, SyntaxKind::TWIG_FILTER)
}

fn parse_twig_indexer(parser: &mut Parser, mut last_node: CompletedMarker) -> CompletedMarker {
    debug_assert!(parser.at(T!["["]));

    // wrap last_node in an operand and create outer marker
    let m = parser.precede(last_node);
    last_node = parser.complete(m, SyntaxKind::TWIG_OPERAND);
    let outer = parser.precede(last_node);

    // bump the opening '['
    parser.bump();

    let index_m = parser.start();
    let mut is_slice = false;
    if parser.at(T![":"]) {
        parser.bump();
        is_slice = true;
    }

    // parse the index expression
    if parse_twig_expression(parser).is_none() && !parser.at(T![":"]) {
        parser.add_error(ParseErrorBuilder::new("twig expression"));
    }

    if parser.at(T![":"]) {
        parser.bump();
        is_slice = true;
        if parse_twig_expression(parser).is_none() {
            parser.add_error(ParseErrorBuilder::new("twig expression"));
        }
    }
    parser.complete(
        index_m,
        match is_slice {
            true => SyntaxKind::TWIG_INDEX_RANGE,
            false => SyntaxKind::TWIG_INDEX,
        },
    );

    parser.expect(T!["]"]);

    // complete the outer marker
    parser.complete(outer, SyntaxKind::TWIG_INDEX_LOOKUP)
}

fn parse_twig_accessor(parser: &mut Parser, mut last_node: CompletedMarker) -> CompletedMarker {
    debug_assert!(parser.at(T!["."]));

    // wrap last_node in an operand and create outer marker
    let m = parser.precede(last_node);
    last_node = parser.complete(m, SyntaxKind::TWIG_OPERAND);
    let outer = parser.precede(last_node);

    // bump the operator
    parser.bump();

    // parse the rhs and wrap it also in an operand
    let m = parser.start();
    if parse_twig_name(parser).is_none() {
        parser.add_error(ParseErrorBuilder::new(
            "twig variable property, key or method",
        ));
    }
    parser.complete(m, SyntaxKind::TWIG_OPERAND);

    // complete the outer marker
    parser.complete(outer, SyntaxKind::TWIG_ACCESSOR)
}

fn parse_twig_function(parser: &mut Parser, mut last_node: CompletedMarker) -> CompletedMarker {
    debug_assert!(parser.at(T!["("]));

    // wrap last_node in an operand and create outer marker
    let m = parser.precede(last_node);
    last_node = parser.complete(m, SyntaxKind::TWIG_OPERAND);
    let outer = parser.precede(last_node);

    // bump the opening '('
    parser.bump();

    // parse any amount of arguments
    let arguments_m = parser.start();
    parse_many(
        parser,
        |p| p.at(T![")"]),
        |p| {
            parse_twig_function_argument(p);
            if p.at(T![","]) {
                p.bump();
            }
        },
    );
    parser.complete(arguments_m, SyntaxKind::TWIG_ARGUMENTS);

    parser.expect(T![")"]);

    // complete the outer marker
    parser.complete(outer, SyntaxKind::TWIG_FUNCTION_CALL)
}

pub(crate) fn parse_twig_function_argument(parser: &mut Parser) -> Option<CompletedMarker> {
    // must be specific here with word followed by equal, because otherwise it could
    // be a normal variable or another function call or something else..
    if parser.at_following(&[T![word], T!["="]]) {
        let named_arg_m = parser.start();
        parser.bump();
        parser.expect(T!["="]);
        parse_twig_expression(parser);
        Some(parser.complete(named_arg_m, SyntaxKind::TWIG_NAMED_ARGUMENT))
    } else {
        parse_twig_expression(parser)
    }
}

pub(crate) fn parse_twig_name(parser: &mut Parser) -> Option<CompletedMarker> {
    // special case to allow for 'same as' and 'divisible by' twig test ('is' / 'is not' operator)
    let is_at_special = parser.at_set(&[T!["same as"], T!["divisible by"]]);
    let token_text = parser.peek_token()?.text;
    if !is_at_special && !TWIG_NAME_REGEX.is_match(token_text) {
        return None;
    }

    let m = parser.start();
    parser.bump_as(SyntaxKind::TK_WORD);
    let m = parser.complete(m, SyntaxKind::TWIG_LITERAL_NAME);
    Some(m)
}

#[cfg(test)]
mod tests {
    use expect_test::expect;

    use crate::parser::check_parse;

    #[test]
    fn parse_twig_string_single_quotes() {
        check_parse(
            r#"{{ 'hel"lo world' }}"#,
            expect![[r#"
                ROOT@0..20
                  TWIG_VAR@0..20
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..17
                      TWIG_LITERAL_STRING@2..17
                        TK_WHITESPACE@2..3 " "
                        TK_SINGLE_QUOTES@3..4 "'"
                        TWIG_LITERAL_STRING_INNER@4..16
                          TK_WORD@4..7 "hel"
                          TK_DOUBLE_QUOTES@7..8 "\""
                          TK_WORD@8..10 "lo"
                          TK_WHITESPACE@10..11 " "
                          TK_WORD@11..16 "world"
                        TK_SINGLE_QUOTES@16..17 "'"
                    TK_WHITESPACE@17..18 " "
                    TK_CLOSE_CURLY_CURLY@18..20 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_double_quotes() {
        check_parse(
            r#"{{ "hel'lo world" }}"#,
            expect![[r#"
                ROOT@0..20
                  TWIG_VAR@0..20
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..17
                      TWIG_LITERAL_STRING@2..17
                        TK_WHITESPACE@2..3 " "
                        TK_DOUBLE_QUOTES@3..4 "\""
                        TWIG_LITERAL_STRING_INNER@4..16
                          TK_WORD@4..7 "hel"
                          TK_SINGLE_QUOTES@7..8 "'"
                          TK_WORD@8..10 "lo"
                          TK_WHITESPACE@10..11 " "
                          TK_WORD@11..16 "world"
                        TK_DOUBLE_QUOTES@16..17 "\""
                    TK_WHITESPACE@17..18 " "
                    TK_CLOSE_CURLY_CURLY@18..20 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_escaped_double_quotes() {
        check_parse(
            r#"{{ "hel\"lo world" }}"#,
            expect![[r#"
                ROOT@0..21
                  TWIG_VAR@0..21
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..18
                      TWIG_LITERAL_STRING@2..18
                        TK_WHITESPACE@2..3 " "
                        TK_DOUBLE_QUOTES@3..4 "\""
                        TWIG_LITERAL_STRING_INNER@4..17
                          TK_WORD@4..7 "hel"
                          TK_BACKWARD_SLASH@7..8 "\\"
                          TK_DOUBLE_QUOTES@8..9 "\""
                          TK_WORD@9..11 "lo"
                          TK_WHITESPACE@11..12 " "
                          TK_WORD@12..17 "world"
                        TK_DOUBLE_QUOTES@17..18 "\""
                    TK_WHITESPACE@18..19 " "
                    TK_CLOSE_CURLY_CURLY@19..21 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_with_leading_and_trailing_trivia() {
        check_parse(
            r#"{{ " , " }}"#,
            expect![[r#"
                ROOT@0..11
                  TWIG_VAR@0..11
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..8
                      TWIG_LITERAL_STRING@2..8
                        TK_WHITESPACE@2..3 " "
                        TK_DOUBLE_QUOTES@3..4 "\""
                        TWIG_LITERAL_STRING_INNER@4..7
                          TK_WHITESPACE@4..5 " "
                          TK_COMMA@5..6 ","
                          TK_WHITESPACE@6..7 " "
                        TK_DOUBLE_QUOTES@7..8 "\""
                    TK_WHITESPACE@8..9 " "
                    TK_CLOSE_CURLY_CURLY@9..11 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_interpolation() {
        check_parse(
            r#"{{ "foo #{1 + 2} baz" }}"#,
            expect![[r##"
                ROOT@0..24
                  TWIG_VAR@0..24
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..21
                      TWIG_LITERAL_STRING@2..21
                        TK_WHITESPACE@2..3 " "
                        TK_DOUBLE_QUOTES@3..4 "\""
                        TWIG_LITERAL_STRING_INNER@4..20
                          TK_WORD@4..7 "foo"
                          TK_WHITESPACE@7..8 " "
                          TWIG_LITERAL_STRING_INTERPOLATION@8..16
                            TK_HASHTAG_OPEN_CURLY@8..10 "#{"
                            TWIG_EXPRESSION@10..15
                              TWIG_BINARY_EXPRESSION@10..15
                                TWIG_EXPRESSION@10..11
                                  TWIG_LITERAL_NUMBER@10..11
                                    TK_NUMBER@10..11 "1"
                                TK_WHITESPACE@11..12 " "
                                TK_PLUS@12..13 "+"
                                TWIG_EXPRESSION@13..15
                                  TWIG_LITERAL_NUMBER@13..15
                                    TK_WHITESPACE@13..14 " "
                                    TK_NUMBER@14..15 "2"
                            TK_CLOSE_CURLY@15..16 "}"
                          TK_WHITESPACE@16..17 " "
                          TK_WORD@17..20 "baz"
                        TK_DOUBLE_QUOTES@20..21 "\""
                    TK_WHITESPACE@21..22 " "
                    TK_CLOSE_CURLY_CURLY@22..24 "}}""##]],
        );
    }

    #[test]
    fn parse_twig_string_interpolation_missing_expression() {
        check_parse(
            r#"{{ "foo #{ } baz" }}"#,
            expect![[r##"
            ROOT@0..20
              TWIG_VAR@0..20
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..17
                  TWIG_LITERAL_STRING@2..17
                    TK_WHITESPACE@2..3 " "
                    TK_DOUBLE_QUOTES@3..4 "\""
                    TWIG_LITERAL_STRING_INNER@4..16
                      TK_WORD@4..7 "foo"
                      TK_WHITESPACE@7..8 " "
                      TWIG_LITERAL_STRING_INTERPOLATION@8..12
                        TK_HASHTAG_OPEN_CURLY@8..10 "#{"
                        TK_WHITESPACE@10..11 " "
                        TK_CLOSE_CURLY@11..12 "}"
                      TK_WHITESPACE@12..13 " "
                      TK_WORD@13..16 "baz"
                    TK_DOUBLE_QUOTES@16..17 "\""
                TK_WHITESPACE@17..18 " "
                TK_CLOSE_CURLY_CURLY@18..20 "}}"
            error at 11..12: expected twig expression but found }"##]],
        );
    }

    #[test]
    fn parse_twig_integer_number() {
        check_parse(
            "{{ 42 }}",
            expect![[r#"
                ROOT@0..8
                  TWIG_VAR@0..8
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..5
                      TWIG_LITERAL_NUMBER@2..5
                        TK_WHITESPACE@2..3 " "
                        TK_NUMBER@3..5 "42"
                    TK_WHITESPACE@5..6 " "
                    TK_CLOSE_CURLY_CURLY@6..8 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_floating_point_number() {
        check_parse(
            "{{ 0.3337 }}",
            expect![[r#"
                ROOT@0..12
                  TWIG_VAR@0..12
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..9
                      TWIG_LITERAL_NUMBER@2..9
                        TK_WHITESPACE@2..3 " "
                        TK_NUMBER@3..9 "0.3337"
                    TK_WHITESPACE@9..10 " "
                    TK_CLOSE_CURLY_CURLY@10..12 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_number_array() {
        check_parse(
            "{{ [1, 2, 3] }}",
            expect![[r#"
            ROOT@0..15
              TWIG_VAR@0..15
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..12
                  TWIG_LITERAL_ARRAY@2..12
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_SQUARE@3..4 "["
                    TWIG_EXPRESSION@4..5
                      TWIG_LITERAL_NUMBER@4..5
                        TK_NUMBER@4..5 "1"
                    TK_COMMA@5..6 ","
                    TWIG_EXPRESSION@6..8
                      TWIG_LITERAL_NUMBER@6..8
                        TK_WHITESPACE@6..7 " "
                        TK_NUMBER@7..8 "2"
                    TK_COMMA@8..9 ","
                    TWIG_EXPRESSION@9..11
                      TWIG_LITERAL_NUMBER@9..11
                        TK_WHITESPACE@9..10 " "
                        TK_NUMBER@10..11 "3"
                    TK_CLOSE_SQUARE@11..12 "]"
                TK_WHITESPACE@12..13 " "
                TK_CLOSE_CURLY_CURLY@13..15 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_array() {
        check_parse(
            r#"{{ ["hello", "trailing", "comma",] }}"#,
            expect![[r#"
            ROOT@0..37
              TWIG_VAR@0..37
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..34
                  TWIG_LITERAL_ARRAY@2..34
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_SQUARE@3..4 "["
                    TWIG_EXPRESSION@4..11
                      TWIG_LITERAL_STRING@4..11
                        TK_DOUBLE_QUOTES@4..5 "\""
                        TWIG_LITERAL_STRING_INNER@5..10
                          TK_WORD@5..10 "hello"
                        TK_DOUBLE_QUOTES@10..11 "\""
                    TK_COMMA@11..12 ","
                    TWIG_EXPRESSION@12..23
                      TWIG_LITERAL_STRING@12..23
                        TK_WHITESPACE@12..13 " "
                        TK_DOUBLE_QUOTES@13..14 "\""
                        TWIG_LITERAL_STRING_INNER@14..22
                          TK_WORD@14..22 "trailing"
                        TK_DOUBLE_QUOTES@22..23 "\""
                    TK_COMMA@23..24 ","
                    TWIG_EXPRESSION@24..32
                      TWIG_LITERAL_STRING@24..32
                        TK_WHITESPACE@24..25 " "
                        TK_DOUBLE_QUOTES@25..26 "\""
                        TWIG_LITERAL_STRING_INNER@26..31
                          TK_WORD@26..31 "comma"
                        TK_DOUBLE_QUOTES@31..32 "\""
                    TK_COMMA@32..33 ","
                    TK_CLOSE_SQUARE@33..34 "]"
                TK_WHITESPACE@34..35 " "
                TK_CLOSE_CURLY_CURLY@35..37 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_null() {
        check_parse(
            "{{ null }}",
            expect![[r#"
            ROOT@0..10
              TWIG_VAR@0..10
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..7
                  TWIG_LITERAL_NULL@2..7
                    TK_WHITESPACE@2..3 " "
                    TK_NULL@3..7 "null"
                TK_WHITESPACE@7..8 " "
                TK_CLOSE_CURLY_CURLY@8..10 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_boolean_true() {
        check_parse(
            "{{ true }}",
            expect![[r#"
            ROOT@0..10
              TWIG_VAR@0..10
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..7
                  TWIG_LITERAL_BOOLEAN@2..7
                    TK_WHITESPACE@2..3 " "
                    TK_TRUE@3..7 "true"
                TK_WHITESPACE@7..8 " "
                TK_CLOSE_CURLY_CURLY@8..10 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_boolean_false() {
        check_parse(
            "{{ false }}",
            expect![[r#"
            ROOT@0..11
              TWIG_VAR@0..11
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..8
                  TWIG_LITERAL_BOOLEAN@2..8
                    TK_WHITESPACE@2..3 " "
                    TK_FALSE@3..8 "false"
                TK_WHITESPACE@8..9 " "
                TK_CLOSE_CURLY_CURLY@9..11 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_number_hash() {
        check_parse(
            "{{ { 1: 'hello' 2: 'world' } }}",
            expect![[r#"
            ROOT@0..31
              TWIG_VAR@0..31
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..28
                  TWIG_LITERAL_HASH@2..28
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_CURLY@3..4 "{"
                    TWIG_LITERAL_HASH_PAIR@4..15
                      TWIG_LITERAL_HASH_KEY@4..6
                        TWIG_LITERAL_NUMBER@4..6
                          TK_WHITESPACE@4..5 " "
                          TK_NUMBER@5..6 "1"
                      TK_COLON@6..7 ":"
                      TWIG_EXPRESSION@7..15
                        TWIG_LITERAL_STRING@7..15
                          TK_WHITESPACE@7..8 " "
                          TK_SINGLE_QUOTES@8..9 "'"
                          TWIG_LITERAL_STRING_INNER@9..14
                            TK_WORD@9..14 "hello"
                          TK_SINGLE_QUOTES@14..15 "'"
                    TWIG_LITERAL_HASH_PAIR@15..26
                      TWIG_LITERAL_HASH_KEY@15..17
                        TWIG_LITERAL_NUMBER@15..17
                          TK_WHITESPACE@15..16 " "
                          TK_NUMBER@16..17 "2"
                      TK_COLON@17..18 ":"
                      TWIG_EXPRESSION@18..26
                        TWIG_LITERAL_STRING@18..26
                          TK_WHITESPACE@18..19 " "
                          TK_SINGLE_QUOTES@19..20 "'"
                          TWIG_LITERAL_STRING_INNER@20..25
                            TK_WORD@20..25 "world"
                          TK_SINGLE_QUOTES@25..26 "'"
                    TK_WHITESPACE@26..27 " "
                    TK_CLOSE_CURLY@27..28 "}"
                TK_WHITESPACE@28..29 " "
                TK_CLOSE_CURLY_CURLY@29..31 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_string_hash() {
        check_parse(
            "{{ { 'hello': 42 'world': 33 } }}",
            expect![[r#"
            ROOT@0..33
              TWIG_VAR@0..33
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..30
                  TWIG_LITERAL_HASH@2..30
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_CURLY@3..4 "{"
                    TWIG_LITERAL_HASH_PAIR@4..16
                      TWIG_LITERAL_HASH_KEY@4..12
                        TWIG_LITERAL_STRING@4..12
                          TK_WHITESPACE@4..5 " "
                          TK_SINGLE_QUOTES@5..6 "'"
                          TWIG_LITERAL_STRING_INNER@6..11
                            TK_WORD@6..11 "hello"
                          TK_SINGLE_QUOTES@11..12 "'"
                      TK_COLON@12..13 ":"
                      TWIG_EXPRESSION@13..16
                        TWIG_LITERAL_NUMBER@13..16
                          TK_WHITESPACE@13..14 " "
                          TK_NUMBER@14..16 "42"
                    TWIG_LITERAL_HASH_PAIR@16..28
                      TWIG_LITERAL_HASH_KEY@16..24
                        TWIG_LITERAL_STRING@16..24
                          TK_WHITESPACE@16..17 " "
                          TK_SINGLE_QUOTES@17..18 "'"
                          TWIG_LITERAL_STRING_INNER@18..23
                            TK_WORD@18..23 "world"
                          TK_SINGLE_QUOTES@23..24 "'"
                      TK_COLON@24..25 ":"
                      TWIG_EXPRESSION@25..28
                        TWIG_LITERAL_NUMBER@25..28
                          TK_WHITESPACE@25..26 " "
                          TK_NUMBER@26..28 "33"
                    TK_WHITESPACE@28..29 " "
                    TK_CLOSE_CURLY@29..30 "}"
                TK_WHITESPACE@30..31 " "
                TK_CLOSE_CURLY_CURLY@31..33 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_named_hash() {
        check_parse(
            "{{ { hello: 42 world: 33 } }}",
            expect![[r#"
            ROOT@0..29
              TWIG_VAR@0..29
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..26
                  TWIG_LITERAL_HASH@2..26
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_CURLY@3..4 "{"
                    TWIG_LITERAL_HASH_PAIR@4..14
                      TWIG_LITERAL_HASH_KEY@4..10
                        TK_WHITESPACE@4..5 " "
                        TK_WORD@5..10 "hello"
                      TK_COLON@10..11 ":"
                      TWIG_EXPRESSION@11..14
                        TWIG_LITERAL_NUMBER@11..14
                          TK_WHITESPACE@11..12 " "
                          TK_NUMBER@12..14 "42"
                    TWIG_LITERAL_HASH_PAIR@14..24
                      TWIG_LITERAL_HASH_KEY@14..20
                        TK_WHITESPACE@14..15 " "
                        TK_WORD@15..20 "world"
                      TK_COLON@20..21 ":"
                      TWIG_EXPRESSION@21..24
                        TWIG_LITERAL_NUMBER@21..24
                          TK_WHITESPACE@21..22 " "
                          TK_NUMBER@22..24 "33"
                    TK_WHITESPACE@24..25 " "
                    TK_CLOSE_CURLY@25..26 "}"
                TK_WHITESPACE@26..27 " "
                TK_CLOSE_CURLY_CURLY@27..29 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_expression_hash() {
        check_parse(
            "{{ { (15): 42 (60): 33 } }}",
            expect![[r#"
            ROOT@0..27
              TWIG_VAR@0..27
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..24
                  TWIG_LITERAL_HASH@2..24
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_CURLY@3..4 "{"
                    TWIG_LITERAL_HASH_PAIR@4..13
                      TWIG_LITERAL_HASH_KEY@4..9
                        TK_WHITESPACE@4..5 " "
                        TK_OPEN_PARENTHESIS@5..6 "("
                        TWIG_EXPRESSION@6..8
                          TWIG_LITERAL_NUMBER@6..8
                            TK_NUMBER@6..8 "15"
                        TK_CLOSE_PARENTHESIS@8..9 ")"
                      TK_COLON@9..10 ":"
                      TWIG_EXPRESSION@10..13
                        TWIG_LITERAL_NUMBER@10..13
                          TK_WHITESPACE@10..11 " "
                          TK_NUMBER@11..13 "42"
                    TWIG_LITERAL_HASH_PAIR@13..22
                      TWIG_LITERAL_HASH_KEY@13..18
                        TK_WHITESPACE@13..14 " "
                        TK_OPEN_PARENTHESIS@14..15 "("
                        TWIG_EXPRESSION@15..17
                          TWIG_LITERAL_NUMBER@15..17
                            TK_NUMBER@15..17 "60"
                        TK_CLOSE_PARENTHESIS@17..18 ")"
                      TK_COLON@18..19 ":"
                      TWIG_EXPRESSION@19..22
                        TWIG_LITERAL_NUMBER@19..22
                          TK_WHITESPACE@19..20 " "
                          TK_NUMBER@20..22 "33"
                    TK_WHITESPACE@22..23 " "
                    TK_CLOSE_CURLY@23..24 "}"
                TK_WHITESPACE@24..25 " "
                TK_CLOSE_CURLY_CURLY@25..27 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_complex_expression_hash() {
        check_parse(
            "{{ { (foo): 'foo', (1 + 1): 'bar', (foo ~ 'b'): 'baz' } }}",
            expect![[r#"
                ROOT@0..58
                  TWIG_VAR@0..58
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..55
                      TWIG_LITERAL_HASH@2..55
                        TK_WHITESPACE@2..3 " "
                        TK_OPEN_CURLY@3..4 "{"
                        TWIG_LITERAL_HASH_PAIR@4..17
                          TWIG_LITERAL_HASH_KEY@4..10
                            TK_WHITESPACE@4..5 " "
                            TK_OPEN_PARENTHESIS@5..6 "("
                            TWIG_EXPRESSION@6..9
                              TWIG_LITERAL_NAME@6..9
                                TK_WORD@6..9 "foo"
                            TK_CLOSE_PARENTHESIS@9..10 ")"
                          TK_COLON@10..11 ":"
                          TWIG_EXPRESSION@11..17
                            TWIG_LITERAL_STRING@11..17
                              TK_WHITESPACE@11..12 " "
                              TK_SINGLE_QUOTES@12..13 "'"
                              TWIG_LITERAL_STRING_INNER@13..16
                                TK_WORD@13..16 "foo"
                              TK_SINGLE_QUOTES@16..17 "'"
                        TK_COMMA@17..18 ","
                        TWIG_LITERAL_HASH_PAIR@18..33
                          TWIG_LITERAL_HASH_KEY@18..26
                            TK_WHITESPACE@18..19 " "
                            TK_OPEN_PARENTHESIS@19..20 "("
                            TWIG_EXPRESSION@20..25
                              TWIG_BINARY_EXPRESSION@20..25
                                TWIG_EXPRESSION@20..21
                                  TWIG_LITERAL_NUMBER@20..21
                                    TK_NUMBER@20..21 "1"
                                TK_WHITESPACE@21..22 " "
                                TK_PLUS@22..23 "+"
                                TWIG_EXPRESSION@23..25
                                  TWIG_LITERAL_NUMBER@23..25
                                    TK_WHITESPACE@23..24 " "
                                    TK_NUMBER@24..25 "1"
                            TK_CLOSE_PARENTHESIS@25..26 ")"
                          TK_COLON@26..27 ":"
                          TWIG_EXPRESSION@27..33
                            TWIG_LITERAL_STRING@27..33
                              TK_WHITESPACE@27..28 " "
                              TK_SINGLE_QUOTES@28..29 "'"
                              TWIG_LITERAL_STRING_INNER@29..32
                                TK_WORD@29..32 "bar"
                              TK_SINGLE_QUOTES@32..33 "'"
                        TK_COMMA@33..34 ","
                        TWIG_LITERAL_HASH_PAIR@34..53
                          TWIG_LITERAL_HASH_KEY@34..46
                            TK_WHITESPACE@34..35 " "
                            TK_OPEN_PARENTHESIS@35..36 "("
                            TWIG_EXPRESSION@36..45
                              TWIG_BINARY_EXPRESSION@36..45
                                TWIG_EXPRESSION@36..39
                                  TWIG_LITERAL_NAME@36..39
                                    TK_WORD@36..39 "foo"
                                TK_WHITESPACE@39..40 " "
                                TK_TILDE@40..41 "~"
                                TWIG_EXPRESSION@41..45
                                  TWIG_LITERAL_STRING@41..45
                                    TK_WHITESPACE@41..42 " "
                                    TK_SINGLE_QUOTES@42..43 "'"
                                    TWIG_LITERAL_STRING_INNER@43..44
                                      TK_WORD@43..44 "b"
                                    TK_SINGLE_QUOTES@44..45 "'"
                            TK_CLOSE_PARENTHESIS@45..46 ")"
                          TK_COLON@46..47 ":"
                          TWIG_EXPRESSION@47..53
                            TWIG_LITERAL_STRING@47..53
                              TK_WHITESPACE@47..48 " "
                              TK_SINGLE_QUOTES@48..49 "'"
                              TWIG_LITERAL_STRING_INNER@49..52
                                TK_WORD@49..52 "baz"
                              TK_SINGLE_QUOTES@52..53 "'"
                        TK_WHITESPACE@53..54 " "
                        TK_CLOSE_CURLY@54..55 "}"
                    TK_WHITESPACE@55..56 " "
                    TK_CLOSE_CURLY_CURLY@56..58 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_nested_hash() {
        check_parse(
            "{{ { outer: { inner: 'hello' } } }}",
            expect![[r#"
            ROOT@0..35
              TWIG_VAR@0..35
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..32
                  TWIG_LITERAL_HASH@2..32
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_CURLY@3..4 "{"
                    TWIG_LITERAL_HASH_PAIR@4..30
                      TWIG_LITERAL_HASH_KEY@4..10
                        TK_WHITESPACE@4..5 " "
                        TK_WORD@5..10 "outer"
                      TK_COLON@10..11 ":"
                      TWIG_EXPRESSION@11..30
                        TWIG_LITERAL_HASH@11..30
                          TK_WHITESPACE@11..12 " "
                          TK_OPEN_CURLY@12..13 "{"
                          TWIG_LITERAL_HASH_PAIR@13..28
                            TWIG_LITERAL_HASH_KEY@13..19
                              TK_WHITESPACE@13..14 " "
                              TK_WORD@14..19 "inner"
                            TK_COLON@19..20 ":"
                            TWIG_EXPRESSION@20..28
                              TWIG_LITERAL_STRING@20..28
                                TK_WHITESPACE@20..21 " "
                                TK_SINGLE_QUOTES@21..22 "'"
                                TWIG_LITERAL_STRING_INNER@22..27
                                  TK_WORD@22..27 "hello"
                                TK_SINGLE_QUOTES@27..28 "'"
                          TK_WHITESPACE@28..29 " "
                          TK_CLOSE_CURLY@29..30 "}"
                    TK_WHITESPACE@30..31 " "
                    TK_CLOSE_CURLY@31..32 "}"
                TK_WHITESPACE@32..33 " "
                TK_CLOSE_CURLY_CURLY@33..35 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_hash_with_omitted_value() {
        check_parse(
            "{{ { value, is, same, as, key } }}",
            expect![[r#"
                ROOT@0..34
                  TWIG_VAR@0..34
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..31
                      TWIG_LITERAL_HASH@2..31
                        TK_WHITESPACE@2..3 " "
                        TK_OPEN_CURLY@3..4 "{"
                        TWIG_LITERAL_HASH_PAIR@4..10
                          TWIG_LITERAL_HASH_KEY@4..10
                            TK_WHITESPACE@4..5 " "
                            TK_WORD@5..10 "value"
                        TK_COMMA@10..11 ","
                        TWIG_LITERAL_HASH_PAIR@11..14
                          TWIG_LITERAL_HASH_KEY@11..14
                            TK_WHITESPACE@11..12 " "
                            TK_WORD@12..14 "is"
                        TK_COMMA@14..15 ","
                        TWIG_LITERAL_HASH_PAIR@15..20
                          TWIG_LITERAL_HASH_KEY@15..20
                            TK_WHITESPACE@15..16 " "
                            TK_WORD@16..20 "same"
                        TK_COMMA@20..21 ","
                        TWIG_LITERAL_HASH_PAIR@21..24
                          TWIG_LITERAL_HASH_KEY@21..24
                            TK_WHITESPACE@21..22 " "
                            TK_WORD@22..24 "as"
                        TK_COMMA@24..25 ","
                        TWIG_LITERAL_HASH_PAIR@25..29
                          TWIG_LITERAL_HASH_KEY@25..29
                            TK_WHITESPACE@25..26 " "
                            TK_WORD@26..29 "key"
                        TK_WHITESPACE@29..30 " "
                        TK_CLOSE_CURLY@30..31 "}"
                    TK_WHITESPACE@31..32 " "
                    TK_CLOSE_CURLY_CURLY@32..34 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_array_with_hash_mixed() {
        check_parse(
            r#"{{ [1, {"foo": "bar"}] }}"#,
            expect![[r#"
            ROOT@0..25
              TWIG_VAR@0..25
                TK_OPEN_CURLY_CURLY@0..2 "{{"
                TWIG_EXPRESSION@2..22
                  TWIG_LITERAL_ARRAY@2..22
                    TK_WHITESPACE@2..3 " "
                    TK_OPEN_SQUARE@3..4 "["
                    TWIG_EXPRESSION@4..5
                      TWIG_LITERAL_NUMBER@4..5
                        TK_NUMBER@4..5 "1"
                    TK_COMMA@5..6 ","
                    TWIG_EXPRESSION@6..21
                      TWIG_LITERAL_HASH@6..21
                        TK_WHITESPACE@6..7 " "
                        TK_OPEN_CURLY@7..8 "{"
                        TWIG_LITERAL_HASH_PAIR@8..20
                          TWIG_LITERAL_HASH_KEY@8..13
                            TWIG_LITERAL_STRING@8..13
                              TK_DOUBLE_QUOTES@8..9 "\""
                              TWIG_LITERAL_STRING_INNER@9..12
                                TK_WORD@9..12 "foo"
                              TK_DOUBLE_QUOTES@12..13 "\""
                          TK_COLON@13..14 ":"
                          TWIG_EXPRESSION@14..20
                            TWIG_LITERAL_STRING@14..20
                              TK_WHITESPACE@14..15 " "
                              TK_DOUBLE_QUOTES@15..16 "\""
                              TWIG_LITERAL_STRING_INNER@16..19
                                TK_WORD@16..19 "bar"
                              TK_DOUBLE_QUOTES@19..20 "\""
                        TK_CLOSE_CURLY@20..21 "}"
                    TK_CLOSE_SQUARE@21..22 "]"
                TK_WHITESPACE@22..23 " "
                TK_CLOSE_CURLY_CURLY@23..25 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_name() {
        check_parse(
            "{{ my_variable }}",
            expect![[r#"
                ROOT@0..17
                  TWIG_VAR@0..17
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..14
                      TWIG_LITERAL_NAME@2..14
                        TK_WHITESPACE@2..3 " "
                        TK_WORD@3..14 "my_variable"
                    TK_WHITESPACE@14..15 " "
                    TK_CLOSE_CURLY_CURLY@15..17 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_token_variable_name() {
        check_parse(
            "{{ and }}",
            expect![[r#"
                ROOT@0..9
                  TWIG_VAR@0..9
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..6
                      TWIG_LITERAL_NAME@2..6
                        TK_WHITESPACE@2..3 " "
                        TK_WORD@3..6 "and"
                    TK_WHITESPACE@6..7 " "
                    TK_CLOSE_CURLY_CURLY@7..9 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_get_attribute_expression() {
        check_parse(
            r#"{{ product.prices.euro }}"#,
            expect![[r#"
                ROOT@0..25
                  TWIG_VAR@0..25
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..22
                      TWIG_ACCESSOR@2..22
                        TWIG_OPERAND@2..17
                          TWIG_ACCESSOR@2..17
                            TWIG_OPERAND@2..10
                              TWIG_LITERAL_NAME@2..10
                                TK_WHITESPACE@2..3 " "
                                TK_WORD@3..10 "product"
                            TK_DOT@10..11 "."
                            TWIG_OPERAND@11..17
                              TWIG_LITERAL_NAME@11..17
                                TK_WORD@11..17 "prices"
                        TK_DOT@17..18 "."
                        TWIG_OPERAND@18..22
                          TWIG_LITERAL_NAME@18..22
                            TK_WORD@18..22 "euro"
                    TK_WHITESPACE@22..23 " "
                    TK_CLOSE_CURLY_CURLY@23..25 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_with_filters() {
        check_parse(
            r#"{{ product.price|striptags|title }}"#,
            expect![[r#"
                ROOT@0..35
                  TWIG_VAR@0..35
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..32
                      TWIG_FILTER@2..32
                        TWIG_OPERAND@2..26
                          TWIG_FILTER@2..26
                            TWIG_OPERAND@2..16
                              TWIG_ACCESSOR@2..16
                                TWIG_OPERAND@2..10
                                  TWIG_LITERAL_NAME@2..10
                                    TK_WHITESPACE@2..3 " "
                                    TK_WORD@3..10 "product"
                                TK_DOT@10..11 "."
                                TWIG_OPERAND@11..16
                                  TWIG_LITERAL_NAME@11..16
                                    TK_WORD@11..16 "price"
                            TK_SINGLE_PIPE@16..17 "|"
                            TWIG_OPERAND@17..26
                              TWIG_LITERAL_NAME@17..26
                                TK_WORD@17..26 "striptags"
                        TK_SINGLE_PIPE@26..27 "|"
                        TWIG_OPERAND@27..32
                          TWIG_LITERAL_NAME@27..32
                            TK_WORD@27..32 "title"
                    TK_WHITESPACE@32..33 " "
                    TK_CLOSE_CURLY_CURLY@33..35 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_array_accessor() {
        check_parse(
            r#"{{ product.prices['eur'] }}"#,
            expect![[r#"
                ROOT@0..27
                  TWIG_VAR@0..27
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..24
                      TWIG_INDEX_LOOKUP@2..24
                        TWIG_OPERAND@2..17
                          TWIG_ACCESSOR@2..17
                            TWIG_OPERAND@2..10
                              TWIG_LITERAL_NAME@2..10
                                TK_WHITESPACE@2..3 " "
                                TK_WORD@3..10 "product"
                            TK_DOT@10..11 "."
                            TWIG_OPERAND@11..17
                              TWIG_LITERAL_NAME@11..17
                                TK_WORD@11..17 "prices"
                        TK_OPEN_SQUARE@17..18 "["
                        TWIG_INDEX@18..23
                          TWIG_EXPRESSION@18..23
                            TWIG_LITERAL_STRING@18..23
                              TK_SINGLE_QUOTES@18..19 "'"
                              TWIG_LITERAL_STRING_INNER@19..22
                                TK_WORD@19..22 "eur"
                              TK_SINGLE_QUOTES@22..23 "'"
                        TK_CLOSE_SQUARE@23..24 "]"
                    TK_WHITESPACE@24..25 " "
                    TK_CLOSE_CURLY_CURLY@25..27 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_nested_array_accessor() {
        check_parse(
            r#"{{ product.prices['eur'][0] }}"#,
            expect![[r#"
                ROOT@0..30
                  TWIG_VAR@0..30
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..27
                      TWIG_INDEX_LOOKUP@2..27
                        TWIG_OPERAND@2..24
                          TWIG_INDEX_LOOKUP@2..24
                            TWIG_OPERAND@2..17
                              TWIG_ACCESSOR@2..17
                                TWIG_OPERAND@2..10
                                  TWIG_LITERAL_NAME@2..10
                                    TK_WHITESPACE@2..3 " "
                                    TK_WORD@3..10 "product"
                                TK_DOT@10..11 "."
                                TWIG_OPERAND@11..17
                                  TWIG_LITERAL_NAME@11..17
                                    TK_WORD@11..17 "prices"
                            TK_OPEN_SQUARE@17..18 "["
                            TWIG_INDEX@18..23
                              TWIG_EXPRESSION@18..23
                                TWIG_LITERAL_STRING@18..23
                                  TK_SINGLE_QUOTES@18..19 "'"
                                  TWIG_LITERAL_STRING_INNER@19..22
                                    TK_WORD@19..22 "eur"
                                  TK_SINGLE_QUOTES@22..23 "'"
                            TK_CLOSE_SQUARE@23..24 "]"
                        TK_OPEN_SQUARE@24..25 "["
                        TWIG_INDEX@25..26
                          TWIG_EXPRESSION@25..26
                            TWIG_LITERAL_NUMBER@25..26
                              TK_NUMBER@25..26 "0"
                        TK_CLOSE_SQUARE@26..27 "]"
                    TK_WHITESPACE@27..28 " "
                    TK_CLOSE_CURLY_CURLY@28..30 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_array_range_accessor() {
        check_parse(
            r#"{{ prices[0:10] }}"#,
            expect![[r#"
                ROOT@0..18
                  TWIG_VAR@0..18
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..15
                      TWIG_INDEX_LOOKUP@2..15
                        TWIG_OPERAND@2..9
                          TWIG_LITERAL_NAME@2..9
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..9 "prices"
                        TK_OPEN_SQUARE@9..10 "["
                        TWIG_INDEX_RANGE@10..14
                          TWIG_EXPRESSION@10..11
                            TWIG_LITERAL_NUMBER@10..11
                              TK_NUMBER@10..11 "0"
                          TK_COLON@11..12 ":"
                          TWIG_EXPRESSION@12..14
                            TWIG_LITERAL_NUMBER@12..14
                              TK_NUMBER@12..14 "10"
                        TK_CLOSE_SQUARE@14..15 "]"
                    TK_WHITESPACE@15..16 " "
                    TK_CLOSE_CURLY_CURLY@16..18 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_array_range_left_accessor() {
        check_parse(
            r#"{{ prices[10:] }}"#,
            expect![[r#"
                ROOT@0..17
                  TWIG_VAR@0..17
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..14
                      TWIG_INDEX_LOOKUP@2..14
                        TWIG_OPERAND@2..9
                          TWIG_LITERAL_NAME@2..9
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..9 "prices"
                        TK_OPEN_SQUARE@9..10 "["
                        TWIG_INDEX_RANGE@10..13
                          TWIG_EXPRESSION@10..12
                            TWIG_LITERAL_NUMBER@10..12
                              TK_NUMBER@10..12 "10"
                          TK_COLON@12..13 ":"
                        TK_CLOSE_SQUARE@13..14 "]"
                    TK_WHITESPACE@14..15 " "
                    TK_CLOSE_CURLY_CURLY@15..17 "}}"
                error at 13..14: expected twig expression but found ]"#]],
        );
    }

    #[test]
    fn parse_twig_variable_array_range_right_accessor() {
        check_parse(
            r#"{{ prices[:10] }}"#,
            expect![[r#"
                ROOT@0..17
                  TWIG_VAR@0..17
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..14
                      TWIG_INDEX_LOOKUP@2..14
                        TWIG_OPERAND@2..9
                          TWIG_LITERAL_NAME@2..9
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..9 "prices"
                        TK_OPEN_SQUARE@9..10 "["
                        TWIG_INDEX_RANGE@10..13
                          TK_COLON@10..11 ":"
                          TWIG_EXPRESSION@11..13
                            TWIG_LITERAL_NUMBER@11..13
                              TK_NUMBER@11..13 "10"
                        TK_CLOSE_SQUARE@13..14 "]"
                    TK_WHITESPACE@14..15 " "
                    TK_CLOSE_CURLY_CURLY@15..17 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_accessor_indexer_and_filter() {
        check_parse(
            r#"{{ product.prices['eur'][0]|title }}"#,
            expect![[r#"
                ROOT@0..36
                  TWIG_VAR@0..36
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..33
                      TWIG_FILTER@2..33
                        TWIG_OPERAND@2..27
                          TWIG_INDEX_LOOKUP@2..27
                            TWIG_OPERAND@2..24
                              TWIG_INDEX_LOOKUP@2..24
                                TWIG_OPERAND@2..17
                                  TWIG_ACCESSOR@2..17
                                    TWIG_OPERAND@2..10
                                      TWIG_LITERAL_NAME@2..10
                                        TK_WHITESPACE@2..3 " "
                                        TK_WORD@3..10 "product"
                                    TK_DOT@10..11 "."
                                    TWIG_OPERAND@11..17
                                      TWIG_LITERAL_NAME@11..17
                                        TK_WORD@11..17 "prices"
                                TK_OPEN_SQUARE@17..18 "["
                                TWIG_INDEX@18..23
                                  TWIG_EXPRESSION@18..23
                                    TWIG_LITERAL_STRING@18..23
                                      TK_SINGLE_QUOTES@18..19 "'"
                                      TWIG_LITERAL_STRING_INNER@19..22
                                        TK_WORD@19..22 "eur"
                                      TK_SINGLE_QUOTES@22..23 "'"
                                TK_CLOSE_SQUARE@23..24 "]"
                            TK_OPEN_SQUARE@24..25 "["
                            TWIG_INDEX@25..26
                              TWIG_EXPRESSION@25..26
                                TWIG_LITERAL_NUMBER@25..26
                                  TK_NUMBER@25..26 "0"
                            TK_CLOSE_SQUARE@26..27 "]"
                        TK_SINGLE_PIPE@27..28 "|"
                        TWIG_OPERAND@28..33
                          TWIG_LITERAL_NAME@28..33
                            TK_WORD@28..33 "title"
                    TK_WHITESPACE@33..34 " "
                    TK_CLOSE_CURLY_CURLY@34..36 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_function_accessor() {
        check_parse(
            r#"{{ product.prices('eur').gross }}"#,
            expect![[r#"
                ROOT@0..33
                  TWIG_VAR@0..33
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..30
                      TWIG_ACCESSOR@2..30
                        TWIG_OPERAND@2..24
                          TWIG_FUNCTION_CALL@2..24
                            TWIG_OPERAND@2..17
                              TWIG_ACCESSOR@2..17
                                TWIG_OPERAND@2..10
                                  TWIG_LITERAL_NAME@2..10
                                    TK_WHITESPACE@2..3 " "
                                    TK_WORD@3..10 "product"
                                TK_DOT@10..11 "."
                                TWIG_OPERAND@11..17
                                  TWIG_LITERAL_NAME@11..17
                                    TK_WORD@11..17 "prices"
                            TK_OPEN_PARENTHESIS@17..18 "("
                            TWIG_ARGUMENTS@18..23
                              TWIG_EXPRESSION@18..23
                                TWIG_LITERAL_STRING@18..23
                                  TK_SINGLE_QUOTES@18..19 "'"
                                  TWIG_LITERAL_STRING_INNER@19..22
                                    TK_WORD@19..22 "eur"
                                  TK_SINGLE_QUOTES@22..23 "'"
                            TK_CLOSE_PARENTHESIS@23..24 ")"
                        TK_DOT@24..25 "."
                        TWIG_OPERAND@25..30
                          TWIG_LITERAL_NAME@25..30
                            TK_WORD@25..30 "gross"
                    TK_WHITESPACE@30..31 " "
                    TK_CLOSE_CURLY_CURLY@31..33 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_variable_deep_function_accessor() {
        check_parse(
            r#"{{ product.prices.gross('eur').gross }}"#,
            expect![[r#"
                ROOT@0..39
                  TWIG_VAR@0..39
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..36
                      TWIG_ACCESSOR@2..36
                        TWIG_OPERAND@2..30
                          TWIG_FUNCTION_CALL@2..30
                            TWIG_OPERAND@2..23
                              TWIG_ACCESSOR@2..23
                                TWIG_OPERAND@2..17
                                  TWIG_ACCESSOR@2..17
                                    TWIG_OPERAND@2..10
                                      TWIG_LITERAL_NAME@2..10
                                        TK_WHITESPACE@2..3 " "
                                        TK_WORD@3..10 "product"
                                    TK_DOT@10..11 "."
                                    TWIG_OPERAND@11..17
                                      TWIG_LITERAL_NAME@11..17
                                        TK_WORD@11..17 "prices"
                                TK_DOT@17..18 "."
                                TWIG_OPERAND@18..23
                                  TWIG_LITERAL_NAME@18..23
                                    TK_WORD@18..23 "gross"
                            TK_OPEN_PARENTHESIS@23..24 "("
                            TWIG_ARGUMENTS@24..29
                              TWIG_EXPRESSION@24..29
                                TWIG_LITERAL_STRING@24..29
                                  TK_SINGLE_QUOTES@24..25 "'"
                                  TWIG_LITERAL_STRING_INNER@25..28
                                    TK_WORD@25..28 "eur"
                                  TK_SINGLE_QUOTES@28..29 "'"
                            TK_CLOSE_PARENTHESIS@29..30 ")"
                        TK_DOT@30..31 "."
                        TWIG_OPERAND@31..36
                          TWIG_LITERAL_NAME@31..36
                            TK_WORD@31..36 "gross"
                    TK_WHITESPACE@36..37 " "
                    TK_CLOSE_CURLY_CURLY@37..39 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_function() {
        check_parse(
            r#"{{ doIt() }}"#,
            expect![[r#"
                ROOT@0..12
                  TWIG_VAR@0..12
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..9
                      TWIG_FUNCTION_CALL@2..9
                        TWIG_OPERAND@2..7
                          TWIG_LITERAL_NAME@2..7
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..7 "doIt"
                        TK_OPEN_PARENTHESIS@7..8 "("
                        TWIG_ARGUMENTS@8..8
                        TK_CLOSE_PARENTHESIS@8..9 ")"
                    TK_WHITESPACE@9..10 " "
                    TK_CLOSE_CURLY_CURLY@10..12 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_function_arguments() {
        check_parse(
            r#"{{ sum(1, 2) }}"#,
            expect![[r#"
                ROOT@0..15
                  TWIG_VAR@0..15
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..12
                      TWIG_FUNCTION_CALL@2..12
                        TWIG_OPERAND@2..6
                          TWIG_LITERAL_NAME@2..6
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..6 "sum"
                        TK_OPEN_PARENTHESIS@6..7 "("
                        TWIG_ARGUMENTS@7..11
                          TWIG_EXPRESSION@7..8
                            TWIG_LITERAL_NUMBER@7..8
                              TK_NUMBER@7..8 "1"
                          TK_COMMA@8..9 ","
                          TWIG_EXPRESSION@9..11
                            TWIG_LITERAL_NUMBER@9..11
                              TK_WHITESPACE@9..10 " "
                              TK_NUMBER@10..11 "2"
                        TK_CLOSE_PARENTHESIS@11..12 ")"
                    TK_WHITESPACE@12..13 " "
                    TK_CLOSE_CURLY_CURLY@13..15 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_function_named_arguments() {
        check_parse(
            r#"{{ sum(a=1, b=2) }}"#,
            expect![[r#"
                ROOT@0..19
                  TWIG_VAR@0..19
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..16
                      TWIG_FUNCTION_CALL@2..16
                        TWIG_OPERAND@2..6
                          TWIG_LITERAL_NAME@2..6
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..6 "sum"
                        TK_OPEN_PARENTHESIS@6..7 "("
                        TWIG_ARGUMENTS@7..15
                          TWIG_NAMED_ARGUMENT@7..10
                            TK_WORD@7..8 "a"
                            TK_EQUAL@8..9 "="
                            TWIG_EXPRESSION@9..10
                              TWIG_LITERAL_NUMBER@9..10
                                TK_NUMBER@9..10 "1"
                          TK_COMMA@10..11 ","
                          TWIG_NAMED_ARGUMENT@11..15
                            TK_WHITESPACE@11..12 " "
                            TK_WORD@12..13 "b"
                            TK_EQUAL@13..14 "="
                            TWIG_EXPRESSION@14..15
                              TWIG_LITERAL_NUMBER@14..15
                                TK_NUMBER@14..15 "2"
                        TK_CLOSE_PARENTHESIS@15..16 ")"
                    TK_WHITESPACE@16..17 " "
                    TK_CLOSE_CURLY_CURLY@17..19 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_function_mixed_named_arguments() {
        check_parse(
            r#"{{ sum(1, b=my_number) }}"#,
            expect![[r#"
                ROOT@0..25
                  TWIG_VAR@0..25
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..22
                      TWIG_FUNCTION_CALL@2..22
                        TWIG_OPERAND@2..6
                          TWIG_LITERAL_NAME@2..6
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..6 "sum"
                        TK_OPEN_PARENTHESIS@6..7 "("
                        TWIG_ARGUMENTS@7..21
                          TWIG_EXPRESSION@7..8
                            TWIG_LITERAL_NUMBER@7..8
                              TK_NUMBER@7..8 "1"
                          TK_COMMA@8..9 ","
                          TWIG_NAMED_ARGUMENT@9..21
                            TK_WHITESPACE@9..10 " "
                            TK_WORD@10..11 "b"
                            TK_EQUAL@11..12 "="
                            TWIG_EXPRESSION@12..21
                              TWIG_LITERAL_NAME@12..21
                                TK_WORD@12..21 "my_number"
                        TK_CLOSE_PARENTHESIS@21..22 ")"
                    TK_WHITESPACE@22..23 " "
                    TK_CLOSE_CURLY_CURLY@23..25 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_function_nested_call() {
        check_parse(
            r#"{{ sum(1, sin(1)) }}"#,
            expect![[r#"
                ROOT@0..20
                  TWIG_VAR@0..20
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..17
                      TWIG_FUNCTION_CALL@2..17
                        TWIG_OPERAND@2..6
                          TWIG_LITERAL_NAME@2..6
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..6 "sum"
                        TK_OPEN_PARENTHESIS@6..7 "("
                        TWIG_ARGUMENTS@7..16
                          TWIG_EXPRESSION@7..8
                            TWIG_LITERAL_NUMBER@7..8
                              TK_NUMBER@7..8 "1"
                          TK_COMMA@8..9 ","
                          TWIG_EXPRESSION@9..16
                            TWIG_FUNCTION_CALL@9..16
                              TWIG_OPERAND@9..13
                                TWIG_LITERAL_NAME@9..13
                                  TK_WHITESPACE@9..10 " "
                                  TK_WORD@10..13 "sin"
                              TK_OPEN_PARENTHESIS@13..14 "("
                              TWIG_ARGUMENTS@14..15
                                TWIG_EXPRESSION@14..15
                                  TWIG_LITERAL_NUMBER@14..15
                                    TK_NUMBER@14..15 "1"
                              TK_CLOSE_PARENTHESIS@15..16 ")"
                        TK_CLOSE_PARENTHESIS@16..17 ")"
                    TK_WHITESPACE@17..18 " "
                    TK_CLOSE_CURLY_CURLY@18..20 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_filter_arguments() {
        check_parse(
            r#"{{ list|join(', ') }}"#,
            expect![[r#"
                ROOT@0..21
                  TWIG_VAR@0..21
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..18
                      TWIG_FILTER@2..18
                        TWIG_OPERAND@2..7
                          TWIG_LITERAL_NAME@2..7
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..7 "list"
                        TK_SINGLE_PIPE@7..8 "|"
                        TWIG_OPERAND@8..18
                          TWIG_LITERAL_NAME@8..12
                            TK_WORD@8..12 "join"
                          TK_OPEN_PARENTHESIS@12..13 "("
                          TWIG_ARGUMENTS@13..17
                            TWIG_EXPRESSION@13..17
                              TWIG_LITERAL_STRING@13..17
                                TK_SINGLE_QUOTES@13..14 "'"
                                TWIG_LITERAL_STRING_INNER@14..16
                                  TK_COMMA@14..15 ","
                                  TK_WHITESPACE@15..16 " "
                                TK_SINGLE_QUOTES@16..17 "'"
                          TK_CLOSE_PARENTHESIS@17..18 ")"
                    TK_WHITESPACE@18..19 " "
                    TK_CLOSE_CURLY_CURLY@19..21 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_double_filter_arguments() {
        check_parse(
            r#"{{ list|join(', ')|trim }}"#,
            expect![[r#"
                ROOT@0..26
                  TWIG_VAR@0..26
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..23
                      TWIG_FILTER@2..23
                        TWIG_OPERAND@2..18
                          TWIG_FILTER@2..18
                            TWIG_OPERAND@2..7
                              TWIG_LITERAL_NAME@2..7
                                TK_WHITESPACE@2..3 " "
                                TK_WORD@3..7 "list"
                            TK_SINGLE_PIPE@7..8 "|"
                            TWIG_OPERAND@8..18
                              TWIG_LITERAL_NAME@8..12
                                TK_WORD@8..12 "join"
                              TK_OPEN_PARENTHESIS@12..13 "("
                              TWIG_ARGUMENTS@13..17
                                TWIG_EXPRESSION@13..17
                                  TWIG_LITERAL_STRING@13..17
                                    TK_SINGLE_QUOTES@13..14 "'"
                                    TWIG_LITERAL_STRING_INNER@14..16
                                      TK_COMMA@14..15 ","
                                      TK_WHITESPACE@15..16 " "
                                    TK_SINGLE_QUOTES@16..17 "'"
                              TK_CLOSE_PARENTHESIS@17..18 ")"
                        TK_SINGLE_PIPE@18..19 "|"
                        TWIG_OPERAND@19..23
                          TWIG_LITERAL_NAME@19..23
                            TK_WORD@19..23 "trim"
                    TK_WHITESPACE@23..24 " "
                    TK_CLOSE_CURLY_CURLY@24..26 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_filter_after_string_with_named_argument() {
        check_parse(
            r#"{{ "now"|date('d/m/Y H:i', timezone="Europe/Paris") }}"#,
            expect![[r#"
                ROOT@0..54
                  TWIG_VAR@0..54
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..51
                      TWIG_FILTER@2..51
                        TWIG_OPERAND@2..8
                          TWIG_LITERAL_STRING@2..8
                            TK_WHITESPACE@2..3 " "
                            TK_DOUBLE_QUOTES@3..4 "\""
                            TWIG_LITERAL_STRING_INNER@4..7
                              TK_WORD@4..7 "now"
                            TK_DOUBLE_QUOTES@7..8 "\""
                        TK_SINGLE_PIPE@8..9 "|"
                        TWIG_OPERAND@9..51
                          TWIG_LITERAL_NAME@9..13
                            TK_WORD@9..13 "date"
                          TK_OPEN_PARENTHESIS@13..14 "("
                          TWIG_ARGUMENTS@14..50
                            TWIG_EXPRESSION@14..25
                              TWIG_LITERAL_STRING@14..25
                                TK_SINGLE_QUOTES@14..15 "'"
                                TWIG_LITERAL_STRING_INNER@15..24
                                  TK_WORD@15..16 "d"
                                  TK_FORWARD_SLASH@16..17 "/"
                                  TK_WORD@17..18 "m"
                                  TK_FORWARD_SLASH@18..19 "/"
                                  TK_WORD@19..20 "Y"
                                  TK_WHITESPACE@20..21 " "
                                  TK_WORD@21..22 "H"
                                  TK_WORD@22..24 ":i"
                                TK_SINGLE_QUOTES@24..25 "'"
                            TK_COMMA@25..26 ","
                            TWIG_NAMED_ARGUMENT@26..50
                              TK_WHITESPACE@26..27 " "
                              TK_WORD@27..35 "timezone"
                              TK_EQUAL@35..36 "="
                              TWIG_EXPRESSION@36..50
                                TWIG_LITERAL_STRING@36..50
                                  TK_DOUBLE_QUOTES@36..37 "\""
                                  TWIG_LITERAL_STRING_INNER@37..49
                                    TK_WORD@37..43 "Europe"
                                    TK_FORWARD_SLASH@43..44 "/"
                                    TK_WORD@44..49 "Paris"
                                  TK_DOUBLE_QUOTES@49..50 "\""
                          TK_CLOSE_PARENTHESIS@50..51 ")"
                    TK_WHITESPACE@51..52 " "
                    TK_CLOSE_CURLY_CURLY@52..54 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_filter_within_binary_comparison() {
        check_parse(
            r#"{{ users|length > 0 }}"#,
            expect![[r#"
                ROOT@0..22
                  TWIG_VAR@0..22
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..19
                      TWIG_BINARY_EXPRESSION@2..19
                        TWIG_EXPRESSION@2..15
                          TWIG_FILTER@2..15
                            TWIG_OPERAND@2..8
                              TWIG_LITERAL_NAME@2..8
                                TK_WHITESPACE@2..3 " "
                                TK_WORD@3..8 "users"
                            TK_SINGLE_PIPE@8..9 "|"
                            TWIG_OPERAND@9..15
                              TWIG_LITERAL_NAME@9..15
                                TK_WORD@9..15 "length"
                        TK_WHITESPACE@15..16 " "
                        TK_GREATER_THAN@16..17 ">"
                        TWIG_EXPRESSION@17..19
                          TWIG_LITERAL_NUMBER@17..19
                            TK_WHITESPACE@17..18 " "
                            TK_NUMBER@18..19 "0"
                    TK_WHITESPACE@19..20 " "
                    TK_CLOSE_CURLY_CURLY@20..22 "}}""#]],
        );
    }

    #[test]
    fn parse_twig_include_function_call() {
        check_parse(
            r#"{{ include('sections/articles/sidebar.html') }}"#,
            expect![[r#"
                ROOT@0..47
                  TWIG_VAR@0..47
                    TK_OPEN_CURLY_CURLY@0..2 "{{"
                    TWIG_EXPRESSION@2..44
                      TWIG_FUNCTION_CALL@2..44
                        TWIG_OPERAND@2..10
                          TWIG_LITERAL_NAME@2..10
                            TK_WHITESPACE@2..3 " "
                            TK_WORD@3..10 "include"
                        TK_OPEN_PARENTHESIS@10..11 "("
                        TWIG_ARGUMENTS@11..43
                          TWIG_EXPRESSION@11..43
                            TWIG_LITERAL_STRING@11..43
                              TK_SINGLE_QUOTES@11..12 "'"
                              TWIG_LITERAL_STRING_INNER@12..42
                                TK_WORD@12..20 "sections"
                                TK_FORWARD_SLASH@20..21 "/"
                                TK_WORD@21..29 "articles"
                                TK_FORWARD_SLASH@29..30 "/"
                                TK_WORD@30..37 "sidebar"
                                TK_DOT@37..38 "."
                                TK_WORD@38..42 "html"
                              TK_SINGLE_QUOTES@42..43 "'"
                        TK_CLOSE_PARENTHESIS@43..44 ")"
                    TK_WHITESPACE@44..45 " "
                    TK_CLOSE_CURLY_CURLY@45..47 "}}""#]],
        );
    }
}
