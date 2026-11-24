-- A one-row `FOR NO KEY UPDATE NOWAIT` probe over a bound key, the shape a test uses
-- to ask whether another transaction holds the row's lock: with none holding the
-- writer it answers the row.
-- @profile-lock
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @param $1 int4 2
-- @test
SELECT 1 FROM lnowait WHERE id = $1 FOR NO KEY UPDATE NOWAIT;
