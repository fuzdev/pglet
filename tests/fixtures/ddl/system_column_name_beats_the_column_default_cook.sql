-- `CheckAttributeNamesTypes` runs inside `heap_create_with_catalog`, still above the
-- cook, so a system-owned column name outranks a bad `DEFAULT` too — in either written
-- order, the reverse spelling `(ctid INT, b INT DEFAULT 'zz')` measuring the same
-- `42701` on both engines. This statement holds the harder arrangement, with the
-- default leading.
-- @expect-error: the system column name outranks the column default cook
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE scndc (b INT DEFAULT 'zz', ctid INT);
