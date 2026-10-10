-- The **silent-wrong** half of the `GREATEST` hole, and the one a fixture with a fully
-- populated seed row cannot reach. `greatest`/`least` filter NULL arguments before
-- folding a common type, so on a row where the offending argument is NULL the fold saw
-- one surviving type, succeeded, and returned a value: `GREATEST(int_col, text_col)`
-- over `(7, NULL)` returned `7`, where Postgres rejects the *query* (`42804`) no matter
-- what the rows contain. A static check is the only thing that sees this.
-- @setup
CREATE TABLE gnr (i4 INT, t TEXT);
INSERT INTO gnr VALUES (7, NULL);
-- @test
-- @expect-error: no common type for integer and text, whatever the row holds (42804)
SELECT GREATEST(i4, t) FROM gnr;
