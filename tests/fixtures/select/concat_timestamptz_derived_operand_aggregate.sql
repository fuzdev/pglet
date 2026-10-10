-- The sibling of `concat_timestamptz_derived_operand.sql` in a **grouping** query, whose
-- expressions the scalar walk evaluates with each aggregate's value read from the
-- grouping: the declared type an operator derives from its operands has to reach it
-- through the aggregate node's own type, which no fixture over a plain column drives.
--
-- `max(ts) + INTERVAL '1 hour'` is the discriminating line: its `timestamptz` is derived
-- from its operands', not read off a leaf, so a node that hands nothing up renders the
-- offset-less `timestamp` text and reddens here.
-- @setup
CREATE TABLE tzg (g INT, ts TIMESTAMPTZ);
INSERT INTO tzg VALUES (1, '2024-06-15 12:30:45.123+00'), (1, '2024-06-15 09:00:00+00');

-- @test
SELECT g,
       max(ts)::text AS cast_of_agg,
       'v=' || max(ts) AS concat_of_agg,
       (max(ts) + INTERVAL '1 hour')::text AS cast_of_agg_arith,
       'v=' || (max(ts) + INTERVAL '1 hour') AS concat_of_agg_arith
FROM tzg
GROUP BY g
