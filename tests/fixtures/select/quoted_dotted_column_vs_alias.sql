-- A column literally named `"a.b"` collides byte-for-byte with the name a join
-- synthesizes for side `a`'s column `b`, so nothing in the name itself tells the
-- two apart. When the synthesized-name tier was consulted first, the quoted column
-- hijacked the qualified reference: `a.b` returned `'WRONG'` (declared `text`)
-- instead of alias `a`'s `b` (`42`, declared `integer`).
--
-- The alias is what `a.` names here, so the alias tier has to win. The declared type
-- is half the assertion — the values `'WRONG'` and `42` are both just strings over
-- the text protocol, so a wrong column that happened to render similarly would
-- compare equal; the `RowDescription` OID (25 vs 23) is what pins the right one.
-- @setup
CREATE TABLE cl ("a.b" TEXT, b INT);
INSERT INTO cl VALUES ('WRONG', 42);

-- @test
SELECT a.b FROM cl AS a;
