-- The same probe rule outside the float family: `timestamp = date` compares as
-- timestamps, so a child at noon does not reference its day. Narrowing the child to a
-- `date` to probe the parent's index would truncate it onto that day and accept.
-- @profile-lock
-- @expect-error: noon is not a date (23503)
-- @expect-message
-- @setup
CREATE TABLE fkpt (d DATE PRIMARY KEY);
INSERT INTO fkpt VALUES ('2024-01-01');
CREATE TABLE fkct (t TIMESTAMP REFERENCES fkpt (d));
INSERT INTO fkct VALUES ('2024-01-01 00:00:00');
-- @test
INSERT INTO fkct VALUES ('2024-01-01 12:00:00');
