-- An `ARRAY[…]` every one of whose elements is an `unknown` is **`text[]`**:
-- `transformArrayExpr` folds it through `select_common_type` before the operator is
-- built, so nothing outside the constructor can retype it. The labels here are *members*
-- and `m IN ('ok','sad')` is an ordinary membership test — so an implementation that
-- read the element as an adoptable `unknown` would accept this, which 18.6 does not.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text, an all-unknown array being text[]
-- @expect-message
SELECT id FROM w WHERE m = ANY(ARRAY['ok', 'sad']);
