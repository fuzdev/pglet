-- A bare string literal compared to an int column inside a `JOIN … ON` is resolved
-- against that column's type, exactly as it is in `WHERE`. Postgres resolves the
-- unknown literal inside operator resolution (`parse_oper.c`), which every clause
-- reaches through the one `transformExpr`, so `ja.id = '1'` is an integer comparison
-- there too. pglet's coercion pass used to run on only six enumerated clauses, and
-- `ON` was not among them — the literal stayed text and the operand gate raised a
-- spurious `42883` (`integer = text`) on a query Postgres runs.
-- @setup
CREATE TABLE ja (id INT, t TEXT);
CREATE TABLE jb (id INT, u TEXT);
INSERT INTO ja VALUES (1, 'x'), (2, 'y');
INSERT INTO jb VALUES (1, 'p'), (2, 'q');
-- @test
SELECT ja.t AS jt, jb.u AS ju FROM ja JOIN jb ON ja.id = jb.id AND ja.id = '1' ORDER BY jt;
