-- An identifier byte glued to the end of a numeric literal is trailing junk
-- (Postgres's `integer_junk` scanner rule): `123abc` is a lex error, not the
-- number `123` followed by a bare column alias `abc`. SQLSTATE 42601.
-- @test
-- @expect-error: trailing junk after numeric literal
SELECT 123abc;
