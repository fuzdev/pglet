-- A relation of the wrong kind anywhere in the list is `42809` ("is not a table"),
-- not `42P01`: the kind is decided on a name that resolves.
-- @setup
CREATE TABLE dwk (a INT);
CREATE SEQUENCE dwk_seq;

-- @expect-error: "dwk_seq" is not a table
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dwk, dwk_seq;
