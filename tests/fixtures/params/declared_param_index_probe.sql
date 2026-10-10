-- A declared `$N` compared against an indexed key column is an index probe on the
-- accelerated build, and answers what the stripped build's row filter answers.
-- @profile-lock
-- @setup
CREATE TABLE dp_probe (id INT PRIMARY KEY, v TEXT);
INSERT INTO dp_probe VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @param-declared $1 int8 2
-- @test
SELECT v FROM dp_probe WHERE id = $1;
