-- `CheckAttributeNamesTypes` runs at the top of `heap_create_with_catalog`, ahead of the
-- `get_relname_relid` test that reports the duplicate relation — so the system column
-- name wins here as the duplicate column does one gate above.
-- @setup
CREATE TABLE scber (q INT);
-- @expect-error: the system column name outranks the existing relation
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE scber (ctid INT);
