-- A `WITH RECURSIVE` body whose second term does **not** self-reference is Postgres's
-- silent degrade to an ordinary `UNION` CTE, so this folds the two terms at
-- `build_set_op` — and that fold read the OID alone, which cannot tell a `mood` from a
-- `text`.
--
-- **It does not reach the recursive type check**, and the sentence here that said it did
-- was wrong: disabling `check_recursive_shape` leaves this fixture green and reddens
-- `enum/recursive_cte_enum_seed_text_term_loud` beside it, which is the genuinely
-- recursive spelling of the same pair. Measured, one mutation. The two are worth keeping
-- apart for exactly that reason: same error, two readers.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
WITH RECURSIVE r(x) AS (SELECT s FROM t UNION ALL SELECT m FROM t WHERE false) SELECT x FROM r;
