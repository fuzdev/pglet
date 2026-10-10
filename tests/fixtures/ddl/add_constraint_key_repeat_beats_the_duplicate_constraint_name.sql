-- The repeat is a parse-analysis verdict on this route too, so it outranks the
-- execution-time checks measured against it — this one, the key columns' own `42703`,
-- a taken index name and the second primary key. The declared name is already taken
-- by a CHECK on the same table, which is a `42710` on both engines when the key list is
-- well-formed.
-- @setup
CREATE TABLE ackn (a INT, b INT);
ALTER TABLE ackn ADD CONSTRAINT ackn_n CHECK (a > 0);
-- @expect-error: the repeated key column outranks the duplicate constraint name
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE ackn ADD CONSTRAINT ackn_n PRIMARY KEY (a, a);
