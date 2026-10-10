-- One `unknown` element beside a typed one does **not** make the array undecidable:
-- `transformArrayExpr` folds the elements through `select_common_type` before the
-- operator is resolved, and an `unknown` contributes nothing to that fold — so
-- `ARRAY[s, 'ok']` is `text[]`, decided by its one typed element, exactly as `ARRAY[s]`
-- is. Letting the literal erase the array's identity made this pair fall back to the OID
-- rule and answer rows.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text over a mixed array
-- @expect-message
-- @profile-lock
SELECT id, (m = ANY(ARRAY[s, 'ok'])) AS a FROM w ORDER BY id;
