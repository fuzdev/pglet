-- An UPDATE whose range is empty — equal ends, both excluded — over an indexed key updates
-- nothing.
-- @profile-lock
-- @setup
CREATE TABLE er (k INT PRIMARY KEY, v TEXT);
INSERT INTO er VALUES (1, 'a'), (3, 'c'), (4, 'd');
-- @test
UPDATE er SET v = 'x' WHERE k > 3 AND k < 3 RETURNING k;
