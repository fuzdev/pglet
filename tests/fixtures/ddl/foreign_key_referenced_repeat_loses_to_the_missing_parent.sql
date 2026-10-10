-- The referenced-list repeat is refused where 18.6 refuses it — inside the constraint's
-- own resolution, once the parent relation has been opened — so this statement's
-- missing parent still answers `42P01`. The bound this fixture sets is that the gate
-- did not migrate up into the parse.
-- @expect-error: the missing parent outranks the repeated referenced column
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE fkmp (a INT, b INT, FOREIGN KEY (a, b) REFERENCES nosuchparent_fkmp (x, x));
