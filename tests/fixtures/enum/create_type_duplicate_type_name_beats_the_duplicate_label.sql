-- The label index is an ordinary catalog index, so its `23505` lands where the insert
-- does — behind the type name's own `42710`, which `TypeCreate` reaches first. Measured
-- on 18.6.
-- @setup
CREATE TYPE ctdt AS ENUM ('a');
-- @expect-error: the existing type name outranks the duplicate label
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TYPE ctdt AS ENUM ('b', 'b');
