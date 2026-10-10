-- An infinite timestamp assigned to a `time` column stores NULL, as `timestamp_time`
-- answers.
-- @setup
CREATE TABLE inftm (id INT, t TIME);
INSERT INTO inftm VALUES (1, timestamp 'infinity'), (2, timestamp '2000-01-01 12:34:56');
-- @test
SELECT id, t FROM inftm ORDER BY id;
