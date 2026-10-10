-- A declared identity `SEQUENCE NAME` over an enum's name: the before-list's sequence
-- meets the type namespace (`42710`), ahead of the table's own heap.
-- @setup
CREATE TYPE tnt8s AS ENUM ('x');
-- @expect-error: a declared identity sequence name over an enum
-- @expect-message
-- @test
CREATE TABLE tnt8 (a INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME tnt8s));
