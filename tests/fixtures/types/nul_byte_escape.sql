-- A NUL byte produced by an octal string escape is rejected: Postgres text can
-- never contain a NUL, so encoding validation raises SQLSTATE 22021
-- (character_not_in_repertoire). The hex form E'\x00' errors identically; a NUL
-- written as a Unicode escape (backslash-u-0000) is instead a 42601
-- invalid-code-point error (both are covered by the lexer unit tests).
-- @test
-- @expect-error: NUL byte in string literal (invalid byte sequence)
SELECT E'a\0b';
