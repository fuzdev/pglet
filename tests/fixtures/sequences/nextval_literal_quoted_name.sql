-- A `regclass` name literal is read by `SplitIdentifierString`, not as the bytes
-- between the quotes: a part beginning `"` is an identifier, so the literal
-- `'"Odd_seq"'` names the relation `Odd_seq`. Reading it verbatim instead named a
-- relation *whose name contains the quote characters* — and because a DDL
-- `DEFAULT nextval('x')` auto-creates `x` on pglet, that was silent rather than loud.
-- The dump quotes the name inside the literal for every sequence whose name is not a
-- safe bare identifier, so both name positions must agree about which relation they
-- mean. The *spelling* is not universal, though: a name containing a backslash takes
-- the escape-string form instead (`nextval(E'"a\\b"'::regclass)`), since that is the
-- one byte whose meaning depends on `standard_conforming_strings`. Two SQL string
-- grammars, one identifier grammar inside them.
-- @setup
CREATE SEQUENCE "Odd_seq" START WITH 40;
-- @test
SELECT nextval('"Odd_seq"') AS bare_quoted, nextval('public."Odd_seq"') AS qualified_quoted;
