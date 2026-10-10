-- A declared `numeric` `$N` ranging over an indexed `integer` key: the probe is inexact,
-- so the index is declined rather than approximated, and both builds answer as 18.6.
-- @profile-lock
-- @setup
CREATE TABLE dp_range (id INT PRIMARY KEY, v TEXT);
INSERT INTO dp_range VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @param-declared $1 numeric 1.5
-- @test
SELECT v FROM dp_range WHERE id > $1 ORDER BY v;
