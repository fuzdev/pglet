-- `x IN (SELECT …)` is 18.6's `x = ANY (subquery)`: `transformSubLink` rewrites the bare
-- `IN`, builds a `PARAM_SUBLINK` carrying the subquery column's own type, and resolves
-- the `=` in parse analysis. The pair had no arm in either operand gate, so it was
-- decided at eval time on `Value` kinds — where an enum is a `Value::Text` like the
-- `text` beside it (`EDGES.md` row 114).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT s FROM w);
