-- The column-definition position takes the refusal too, in the ordered column walk:
-- the statement is rejected outright rather than landing a `boolean` column the
-- statement did not ask for.
-- @expect-error: bool takes no type modifier at a column definition
-- @expect-message
-- @test
CREATE TABLE tmnc (a bool(3));
