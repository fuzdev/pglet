-- The subquery side is **never** an `unknown` the operator's resolution can convert: its
-- target-list entry is resolved first and the `PARAM_SUBLINK` carries that type, so a bare
-- `'ok'` inside the subquery is `text` and the pair has no operator — even though `'ok'`
-- is a member and `m IN ('ok')` is an ordinary membership test. A gate that converted
-- *both* sides, as the sibling-`Expr` positions do, would accept this.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text, the subquery column being text and not unknown
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT 'ok');
