use crate::ast::*;
use crate::error::{DynamicParseError, TwigParsingErrorInformation};
use crate::parser::html::{html_comment, html_complete_tag, html_plain_text};
use crate::parser::twig::{twig_comment, twig_complete_block, twig_parent_call};
use crate::parser::vue::vue_block;
use nom::branch::alt;
use nom::character::complete::multispace1;
use nom::multi::many1;
use nom::Parser;

pub(crate) type Input<'a> = &'a str;
pub(crate) type IResult<'a, O> = nom::IResult<Input<'a>, O, TwigParsingErrorInformation<Input<'a>>>;

/// This is used mainly as the [dynamic_context] combinator, to add user friendly information
/// to errors when backtracking through a parse tree.
/// It works by executing the callback function context_creator to get a dynamic String,
/// which is then stored in the error as context
/// (this execution only happens in the case of parsing errors).
pub(crate) fn dynamic_context<I: Clone, C, E: DynamicParseError<I>, F, O>(
    context_creator: C,
    mut f: F,
) -> impl FnMut(I) -> nom::IResult<I, O, E>
where
    C: Fn() -> String,
    F: Parser<I, O, E>,
{
    move |i: I| match f.parse(i.clone()) {
        Ok(o) => Ok(o),
        Err(nom::Err::Incomplete(i)) => Err(nom::Err::Incomplete(i)),
        Err(nom::Err::Error(e)) => Err(nom::Err::Error(E::add_dynamic_context(
            i,
            context_creator(),
            e,
        ))),
        Err(nom::Err::Failure(e)) => Err(nom::Err::Failure(E::add_dynamic_context(
            i,
            context_creator(),
            e,
        ))),
    }
}

// whitespace because it matters in rendering!: https://prettier.io/blog/2018/11/07/1.15.0.html
pub(crate) fn some_whitespace(input: Input) -> IResult<HtmlNode> {
    let (remainder, _) = multispace1(input)?;

    Ok((remainder, HtmlNode::Whitespace))
}

pub(crate) fn document_node(input: Input) -> IResult<HtmlNode> {
    alt((
        some_whitespace,
        html_comment, //html comment must match before html tag because both can start with <!...
        html_complete_tag,
        twig_complete_block,
        vue_block,
        twig_parent_call,
        twig_comment,
        html_plain_text,
    ))(input)
}

pub(crate) fn document_node_all(input: Input) -> IResult<HtmlNode> {
    let (remaining, children) = many1(document_node)(&input)?;

    Ok((remaining, HtmlNode::Root(children)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_some_vue_template() {
        let res = document_node(
            "<sw-button-group
                v-if=\"startButtonVisible\"
                :splitButton=\"true\">

            <sw-button variant=\"primary\"
                       :disabled=\"startButtonDisabled\"
                       @click=\"onStartButtonClick\">
                {{ $tc('swag-migration.index.confirmAbortDialog.hint') }}
            </sw-button>

            <sw-context-button :disabled=\"isLoading\">
                <template slot=\"button\">

                    <sw-button square
                               variant=\"primary\"
                               :disabled=\"isLoading\">
                        <sw-icon name=\"small-arrow-medium-down\" size=\"16\"></sw-icon>
                    </sw-button>
                </template>

                <sw-context-menu-item @click=\"onSaveButtonClick\"
                                      :disabled=\"isLoading\">
                    {{ $tc('swag-migration.index.confirmAbortDialog.hint') }}
                </sw-context-menu-item>
            </sw-context-button>

        </sw-button-group>",
        );

        assert!(res.is_ok());
    }

    #[test]
    fn test_whitespace_between_nodes() {
        let res = document_node_all(
            "<h2>
    HelloWorld<span>asdf</span>
</h2>",
        );

        assert_eq!(
            res,
            Ok((
                "",
                HtmlNode::Root(vec![HtmlNode::Tag(HtmlTag {
                    name: "h2".to_string(),
                    children: vec![
                        HtmlNode::Whitespace,
                        HtmlNode::Plain(HtmlPlain {
                            plain: "HelloWorld".to_string()
                        }),
                        HtmlNode::Tag(HtmlTag {
                            name: "span".to_string(),
                            children: vec![HtmlNode::Plain(HtmlPlain {
                                plain: "asdf".to_string()
                            })],
                            ..Default::default()
                        }),
                        HtmlNode::Whitespace
                    ],
                    ..Default::default()
                })])
            ))
        );
    }

    #[test]
    fn test_no_whitespace_between_nodes() {
        let res = document_node_all("<h2>HelloWorld<span>asdf</span></h2>");

        assert_eq!(
            res,
            Ok((
                "",
                HtmlNode::Root(vec![HtmlNode::Tag(HtmlTag {
                    name: "h2".to_string(),
                    children: vec![
                        HtmlNode::Plain(HtmlPlain {
                            plain: "HelloWorld".to_string()
                        }),
                        HtmlNode::Tag(HtmlTag {
                            name: "span".to_string(),
                            children: vec![HtmlNode::Plain(HtmlPlain {
                                plain: "asdf".to_string()
                            })],
                            ..Default::default()
                        })
                    ],
                    ..Default::default()
                })])
            ))
        );
    }

    #[test]
    fn test_whitespace_with_plain_text() {
        let res = document_node_all(
            "<h2>
    Hello World
    this is <strong>some</strong> text
    <span>!!!</span>
</h2>",
        );

        assert_eq!(
            res,
            Ok((
                "",
                HtmlNode::Root(vec![HtmlNode::Tag(HtmlTag {
                    name: "h2".to_string(),
                    children: vec![
                        HtmlNode::Whitespace,
                        HtmlNode::Plain(HtmlPlain {
                            plain: "Hello World".to_string()
                        }),
                        HtmlNode::Whitespace,
                        HtmlNode::Plain(HtmlPlain {
                            plain: "this is".to_string()
                        }),
                        HtmlNode::Whitespace,
                        HtmlNode::Tag(HtmlTag {
                            name: "strong".to_string(),
                            children: vec![HtmlNode::Plain(HtmlPlain {
                                plain: "some".to_string()
                            })],
                            ..Default::default()
                        }),
                        HtmlNode::Whitespace,
                        HtmlNode::Plain(HtmlPlain {
                            plain: "text".to_string()
                        }),
                        HtmlNode::Whitespace,
                        HtmlNode::Tag(HtmlTag {
                            name: "span".to_string(),
                            children: vec![HtmlNode::Plain(HtmlPlain {
                                plain: "!!!".to_string()
                            })],
                            ..Default::default()
                        }),
                        HtmlNode::Whitespace
                    ],
                    ..Default::default()
                })])
            ))
        );
    }
}
