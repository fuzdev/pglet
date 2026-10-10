-- Position: the heap is created before its constraint indexes, so the type namespace's
-- `42710` outranks a constraint index name already taken by a relation (`42P07`).
-- @setup
CREATE TABLE tnt12x (a INT);
CREATE TYPE tnt12 AS ENUM ('x');
-- @expect-error: the type collision outranks the taken constraint index name
-- @expect-message
-- @no-profile-lock: the @test is DDL whose refusal lands before any index is built or planned
-- @test
CREATE TABLE tnt12 (a INT, CONSTRAINT tnt12x PRIMARY KEY (a));
