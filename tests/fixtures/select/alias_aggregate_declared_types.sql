-- The **aggregate node** builds its output descriptors on its own arm of the same
-- walk, so a fix confined to the projection arm leaves this red. An aggregate's
-- result type follows its argument's, and the argument here is an expression rather
-- than a bare column — the bare-column case already resolved against the input
-- descriptors, which is why `qualified_aggregate_declared_types` next door stayed
-- green while `max(m.b + 0)` declared `int4` over an eight-byte value.
--
-- `grp` is the polarity pin: a grouping column that resolved correctly before and
-- must still declare `int2`.
--
-- `sum` is deliberately not used here: pglet declares `sum(bigint)` as `int8` where
-- 18.6 declares `numeric`, on **both** legs and with or without an alias, so a `sum`
-- column would redden this fixture for a defect it does not own.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
SELECT m.s AS grp,               -- int2 — polarity pin
       max(m.b + 0) AS agg_max,  -- int8 (was int4)
       min(m.b + 0) AS agg_min   -- int8 (was int4)
FROM mw m GROUP BY m.s;
