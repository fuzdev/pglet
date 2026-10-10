-- `ADD COLUMN … SERIAL` names its sequence through the same gate.
-- @setup
CREATE TYPE tnt10_b_seq AS ENUM ('x');
CREATE TABLE tnt10 (a INT);
-- @expect-error: the added column's sequence over an enum
-- @expect-message
-- @test
ALTER TABLE tnt10 ADD COLUMN b SERIAL;
