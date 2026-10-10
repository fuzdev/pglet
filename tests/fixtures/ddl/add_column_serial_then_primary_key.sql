-- A primary key added over an added serial column, and the ordered read it serves.
-- @profile-lock
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('a'), ('b');
ALTER TABLE acs ADD COLUMN id SERIAL;
ALTER TABLE acs ADD PRIMARY KEY (id);
INSERT INTO acs (v) VALUES ('c');
-- @test
SELECT id, v FROM acs ORDER BY id LIMIT 2;
