-- Binding and the positional range check **interleave per key**, in source order, as 18.6
-- resolves them: measured there, this is `42703` and its reversed twin
-- (`select/no_from_order_by_bad_position_before_a_bogus_key_loud.sql`) is `42P10`. No
-- arrangement of one whole-list binding pass against one whole-list range pass yields
-- both, which is why `plan_no_from_order_limit` runs the two inside one per-key closure —
-- the same shape the no-FROM `DISTINCT ON` gate and the blanket's replacement already use.
-- @expect-error: column "zzz" does not exist (42703)
-- @test
SELECT 1 AS q ORDER BY zzz, 9;
