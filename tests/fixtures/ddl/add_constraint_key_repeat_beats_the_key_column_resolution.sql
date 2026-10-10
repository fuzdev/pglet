-- The route asymmetry, stated as the one spelling that reads it off: on `ALTER TABLE …
-- ADD CONSTRAINT` a key naming no column is still `42701` when it is repeated, because
-- the resolution 18.6 would raise `42703` from is not in that pass at all. The
-- `CREATE TABLE` spelling of this very list is `42703`: `CREATE TABLE t (b INT,
-- PRIMARY KEY (nosuch, nosuch))` is the direct comparison and is `42703` on both
-- engines.
-- @setup
CREATE TABLE ackr (a INT);
-- @expect-error: the repeat is tested before the key is resolved on this route
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE ackr ADD CONSTRAINT ackr_pk PRIMARY KEY (nosuch, nosuch);
