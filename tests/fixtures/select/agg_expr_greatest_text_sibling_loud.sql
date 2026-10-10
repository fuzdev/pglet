-- `greatest` over an aggregate and a `text` column has no common type: 18.6's `42804`.
-- The aggregate argument is typed when the variadic unifies, so it reaches the
-- unification gate uncast instead of being cast to `text` and compared.
-- @setup
CREATE TABLE r (i INT, tx TEXT);
INSERT INTO r VALUES (7, '7'), (3, 'a');
-- @expect-error: GREATEST types bigint and text cannot be matched
-- @test
SELECT greatest(sum(i), tx) FROM r GROUP BY tx;
