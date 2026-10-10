-- And within `numerictypmodin`'s two-component branch the precision is checked before the
-- scale (by the `make_numeric_typmod_safe` both branches call), so a modifier wrong in
-- both names the precision. The mirror of the pair above, one step down.
-- @expect-error: the precision window outranks the scale window
-- @expect-message
-- @test
SELECT 1.5::numeric(0,1001) AS never_reached;
