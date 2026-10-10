-- The set-operation door, where the `unknown` side's coercion is pglet's own
-- `coerce_unknown_entry` — which wrote a cast to `text`, 18.6's *other* type, and so
-- proved nothing about membership.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT m FROM t UNION ALL SELECT 'zzz';
