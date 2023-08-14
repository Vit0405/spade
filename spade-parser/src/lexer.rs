use logos::Logos;

use num::BigInt;

#[derive(Debug, PartialEq, Clone)]
pub enum LiteralKind {
    Signed,
    Unsigned,
}

fn parse_int(slice: &str, radix: u32) -> (BigInt, LiteralKind) {
    let lower = slice.to_ascii_lowercase();
    let cleaned = lower.replace('_', "").replace("u", "").replace("U", "");

    let signed = if lower.ends_with('u') {
        LiteralKind::Unsigned
    } else {
        LiteralKind::Signed
    };

    (
        BigInt::parse_bytes(&cleaned.as_bytes(), radix).unwrap(),
        signed,
    )
}

#[derive(Logos, Debug, PartialEq, Clone)]
pub enum TokenKind {
    // Unholy regex for unicode identifiers. Stolen from Repnop who stole it from Evrey
    #[regex(r#"(?x:
        [\p{XID_Start}_]
        \p{XID_Continue}*
        (\u{3F} | \u{21} | (\u{3F}\u{21}) | \u{2048})? # ? ! ?! ⁈
    )"#, |lex| lex.slice().to_string())]
    Identifier(String),

    #[regex(r"-?[0-9][0-9_]*[uU]?", |lex| {
        parse_int(lex.slice(), 10)
    })]
    Integer((BigInt, LiteralKind)),
    #[regex(r"-?0x[0-9A-Fa-f][0-9_A-Fa-f]*[uU]?", |lex| {
        parse_int(&lex.slice()[2..], 16)
    })]
    HexInteger((BigInt, LiteralKind)),
    #[regex(r"-?0b[0-1][0-1_]*[uU]?", |lex| {
        parse_int(&lex.slice()[2..], 2)
    })]
    BinInteger((BigInt, LiteralKind)),

    #[token("true")]
    True,
    #[token("false")]
    False,

    // Keywords
    #[token("reg")]
    Reg,
    #[token("let")]
    Let,
    #[token("decl")]
    Decl,
    #[token("inst")]
    Instance,
    #[token("reset")]
    Reset,
    #[token("initial")]
    Initial,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("match")]
    Match,
    #[token("set")]
    Set,

    #[token("pipeline")]
    Pipeline,
    #[token("stage")]
    Stage,
    #[token("entity")]
    Entity,
    #[token("trait")]
    Trait,
    #[token("impl")]
    Impl,
    #[token("for")]
    For,
    #[token("fn")]
    Function,
    #[token("enum")]
    Enum,
    #[token("struct")]
    Struct,
    #[token("port")]
    Port,
    #[token("mod")]
    Mod,
    #[token("use")]
    Use,
    #[token("as")]
    As,
    #[token("assert")]
    Assert,
    #[token("mut")]
    Mut,

    #[token("$config")]
    ComptimeConfig,
    #[token("$if")]
    ComptimeIf,
    #[token("$else")]
    ComptimeElse,

    // Math operators
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Asterisk,
    #[token("/")]
    Slash,
    #[token("==")]
    Equals,
    #[token("!=")]
    NotEquals,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token(">>>")]
    ArithmeticRightShift,
    #[token(">>")]
    RightShift,
    #[token("<<")]
    LeftShift,
    #[token("||")]
    LogicalOr,
    #[token("&&")]
    LogicalAnd,
    #[token("^^")]
    LogicalXor,
    #[token("&")]
    Ampersand,
    #[token("|")]
    BitwiseOr,
    #[token("!")]
    Not,
    #[token("^")]
    BitwiseXor,
    #[token("~")]
    Tilde,
    #[token("`")]
    InfixOperatorSeparator,
    #[token("'")]
    SingleQuote,

    // Other operators
    #[token("=")]
    Assignment,

    #[token("(")]
    OpenParen,
    #[token(")")]
    CloseParen,

    #[token("{")]
    OpenBrace,
    #[token("}")]
    CloseBrace,

    #[token("[")]
    OpenBracket,
    #[token("]")]
    CloseBracket,

    #[token("=>")]
    FatArrow,
    #[token("->")]
    SlimArrow,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token("::")]
    PathSeparator,
    #[token("#")]
    Hash,
    #[token("$")]
    Dollar,

    #[token("__builtin__")]
    Builtin,

    /// Ignoring whitespace
    #[regex("[ \t\n\r]", logos::skip)]
    Whitespace,

    #[regex("//[^\n]*\n", logos::skip)]
    Comment,

    Eof,
}

impl TokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenKind::Identifier(_) => "identifier",
            TokenKind::Integer(_) => "integer",
            TokenKind::HexInteger(_) => "hexadecimal integer",
            TokenKind::BinInteger(_) => "binary integer",
            TokenKind::True => "true",
            TokenKind::False => "false",

            TokenKind::Let => "let",
            TokenKind::Reg => "reg",
            TokenKind::Decl => "decl",
            TokenKind::Entity => "entity",
            TokenKind::Pipeline => "pipeline",
            TokenKind::Stage => "stage",
            TokenKind::Instance => "inst",
            TokenKind::Reset => "reset",
            TokenKind::Initial => "initial",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::Match => "match",
            TokenKind::Impl => "impl",
            TokenKind::Trait => "trait",
            TokenKind::For => "for",
            TokenKind::Function => "fn",
            TokenKind::Enum => "enum",
            TokenKind::Struct => "struct",
            TokenKind::Port => "port",
            TokenKind::Mod => "mod",
            TokenKind::As => "as",
            TokenKind::Use => "use",
            TokenKind::Assert => "assert",
            TokenKind::Set => "set",
            TokenKind::Mut => "mut",

            TokenKind::ComptimeConfig => "$config",
            TokenKind::ComptimeIf => "$if",
            TokenKind::ComptimeElse => "$else",

            TokenKind::Assignment => "=",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Asterisk => "*",
            TokenKind::Slash => "/",
            TokenKind::Equals => "==",
            TokenKind::NotEquals => "!=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::Le => "<=",
            TokenKind::Ge => ">=",
            TokenKind::LeftShift => "<<",
            TokenKind::RightShift => ">>",
            TokenKind::ArithmeticRightShift => ">>>",
            TokenKind::LogicalOr => "||",
            TokenKind::LogicalAnd => "&&",
            TokenKind::LogicalXor => "^^",
            TokenKind::Ampersand => "&",
            TokenKind::BitwiseOr => "|",
            TokenKind::Not => "!",
            TokenKind::Tilde => "~",
            TokenKind::BitwiseXor => "^",
            TokenKind::InfixOperatorSeparator => "`",

            TokenKind::OpenParen => "(",
            TokenKind::CloseParen => ")",
            TokenKind::OpenBrace => "{",
            TokenKind::CloseBrace => "}",
            TokenKind::OpenBracket => "[",
            TokenKind::CloseBracket => "]",

            TokenKind::FatArrow => "=>",
            TokenKind::SlimArrow => "->",
            TokenKind::Semi => ";",
            TokenKind::Colon => ":",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",
            TokenKind::PathSeparator => "::",
            TokenKind::SingleQuote => "'",

            TokenKind::Hash => "#",
            TokenKind::Dollar => "$",

            TokenKind::Builtin => "__builtin__",

            TokenKind::Eof => "end of file",

            TokenKind::Whitespace => "whitespace",
            TokenKind::Comment => "comment",
        }
    }

    pub fn is_identifier(&self) -> bool {
        matches!(self, TokenKind::Identifier(_))
    }
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            TokenKind::Integer(_) | TokenKind::HexInteger(_) | TokenKind::BinInteger(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use spade_common::num_ext::InfallibleToBigInt;

    use super::*;

    #[test]
    fn identifiers_work() {
        let mut lex = TokenKind::lexer("abc123_");

        assert_eq!(
            lex.next(),
            Some(Ok(TokenKind::Identifier("abc123_".to_string())))
        );
    }

    #[test]
    fn integer_literals_work() {
        let mut lex = TokenKind::lexer("123");

        assert_eq!(
            lex.next(),
            Some(Ok(TokenKind::Integer((
                123.to_bigint(),
                LiteralKind::Signed
            ))))
        );
        assert_eq!(lex.next(), None);
    }

    #[test]
    fn hex_array() {
        let mut lex = TokenKind::lexer("[0x45]");
        assert_eq!(lex.next(), Some(Ok(TokenKind::OpenBracket)));
        assert_eq!(
            lex.next(),
            Some(Ok(TokenKind::HexInteger((
                0x45.to_bigint(),
                LiteralKind::Signed
            ))))
        );
        assert_eq!(lex.next(), Some(Ok(TokenKind::CloseBracket)));
        assert_eq!(lex.next(), None);
    }

    #[test]
    fn invalid_hex_is_not_hex() {
        let mut lex = TokenKind::lexer("0xg");
        assert_eq!(
            lex.next(),
            Some(Ok(TokenKind::Integer((0.to_bigint(), LiteralKind::Signed))))
        );
        assert_eq!(
            lex.next(),
            Some(Ok(TokenKind::Identifier("xg".to_string())))
        );
        assert_eq!(lex.next(), None);
    }
}
