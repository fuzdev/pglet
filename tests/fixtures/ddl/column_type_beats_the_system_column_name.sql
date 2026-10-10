-- The same position read against the system-column gate, which sits one layer further in
-- than the duplicate column and therefore loses to the type walk too.
-- @expect-error: the refused char length outranks the system column name
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctbsc (a CHAR(0), ctid INT);
