-- A row a CHECK over `left` refuses: 23514.
-- @setup
CREATE TABLE lv (t TEXT CHECK (left(t, 1) <> 'x'));
-- @test
-- @expect-error: violates check constraint
INSERT INTO lv VALUES ('xyz');
