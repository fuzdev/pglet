-- Set-op columns unify across type families, not just the numeric family: text and
-- varchar unify to text, and date and timestamp unify to timestamp (the wider). The
-- date value casts to a timestamp at midnight, and both output columns carry the
-- unified type / wire label.
-- @setup
CREATE TABLE tuf (t TEXT, v VARCHAR(10), d DATE, ts TIMESTAMP);
INSERT INTO tuf VALUES ('a', 'bb', '2020-01-01', '2020-06-15 12:30:00');
-- @test
SELECT t AS c1, d AS c2 FROM tuf
UNION ALL
SELECT v AS c1, ts AS c2 FROM tuf
ORDER BY c1;
