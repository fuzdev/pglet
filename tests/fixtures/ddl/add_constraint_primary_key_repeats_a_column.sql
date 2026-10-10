-- The `ALTER TABLE … ADD CONSTRAINT` route takes the same repeat test at a **different
-- position**: `transformIndexConstraint` skips its key resolution there (`!cxt->isalter`
-- leaves it to `DefineIndex`), so the repeat is reached first and a key naming no column
-- at all still answers `42701` —
-- `add_constraint_key_repeat_beats_the_key_column_resolution.sql` pins that, and it is
-- the inverse of the `CREATE TABLE` route's order.
-- @setup
CREATE TABLE acpr (a INT);
-- @expect-error: a repeated column in a primary key
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acpr ADD CONSTRAINT acpr_pk PRIMARY KEY (a, a);
