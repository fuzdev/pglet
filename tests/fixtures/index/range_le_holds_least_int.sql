-- An upper bound at `integer`'s least value over a plain index: that key's row.
-- @profile-lock
-- @setup
CREATE TABLE lk (c INT, v TEXT);
CREATE INDEX lk_c ON lk (c);
INSERT INTO lk VALUES (NULL, 'null'), (-2147483648, 'least'), (0, 'zero');
-- @test
SELECT v FROM lk WHERE c <= -2147483648;
