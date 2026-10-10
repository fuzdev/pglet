-- A column-level and a table-level declared name share **one** pool: `pg_constraint`'s
-- unique index is `(conrelid, conname)` and knows nothing about which spelling the user
-- reached for, so the inline `CONSTRAINT n` and the table-level `CONSTRAINT n` collide.
-- Constraint 2's name still beats constraint 3's expression, one scale up from
-- `create_table_check_expr_beats_own_dup_name.sql`.
--
-- This arrangement is deliberately order-insensitive — the column-level constraint is
-- also the *first written*, so it claims `n` under any fold. Which element is resolved
-- first when the two spellings interleave is
-- `create_table_table_level_fk_named_before_inline.sql`'s subject; the assertion here is
-- only that they share a pool.
-- @expect-error: constraint n for relation ctmx already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctmx (a INT CONSTRAINT n CHECK (a > 0), CONSTRAINT n CHECK (a > 1),
                   CONSTRAINT m CHECK (a LIKE 'x'));
