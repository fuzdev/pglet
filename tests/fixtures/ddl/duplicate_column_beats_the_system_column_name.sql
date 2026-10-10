-- Another same-SQLSTATE pair: `MergeAttributes` runs before
-- `heap_create_with_catalog`, so a repeat outranks a system column name **whichever is
-- written first** — measured on 18.6 in both orders. This statement holds the harder
-- one, with the system name leading.
-- @expect-error: the duplicate column outranks the system column name
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE dcbsc (ctid INT, a INT, a INT);
