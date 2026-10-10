-- The positive twin of `select/from_values_cell_no_common_type_loud.sql`: the cells go
-- through the full no-columns *resolution*, not just the gate, so an unknown literal
-- still adopts its sibling's concrete type. Gating the unresolved cell instead would
-- reject this as `integer = text`, and leaving it unresolved (as before) let the
-- comparison reach eval untyped and fail there.
-- @test
SELECT * FROM (VALUES (1 = '1'), (2 = '3')) v(c);
