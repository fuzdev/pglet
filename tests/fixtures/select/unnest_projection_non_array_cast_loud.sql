-- A **cast** argument reaches the gate through the same reader as a column
-- (`arg_known_type`'s `Cast` arm), so the refusal is the argument's *target* type and
-- not the column's. `unnest(NULL::int)` is the same shape with no column at all — 18.6
-- refuses both `42883`, while `unnest(NULL::int[])` answers zero rows on both engines.
-- @setup
CREATE TABLE unnest_na_cast (i INT);
-- @test
-- @expect-error: no unnest(text)
SELECT unnest(i::TEXT) FROM unnest_na_cast;
