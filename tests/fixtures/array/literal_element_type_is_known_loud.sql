-- An `ARRAY[…]` **literal** used as a branch had no static type at all, so the enclosing
-- set unified to a sibling's *scalar* type and cast the inner array to it. The failure
-- then surfaced at eval as an impossible cast (`cannot cast type Some(Int4Array) to
-- Bool`, which also leaked a `Debug`-formatted `Option` into a user-facing message)
-- rather than at plan time as the no-common-type error Postgres reports. An array
-- constructor is never *unknown* in Postgres — an all-unknown element set resolves to
-- `text`, so `ARRAY[…]` always has a type one dimension up from its elements'.
-- @expect-error: ARRAY types boolean and integer[] cannot be matched (42804)
-- @test
SELECT ARRAY[true, ARRAY[1,2]];
