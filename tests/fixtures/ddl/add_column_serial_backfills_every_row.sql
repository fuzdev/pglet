-- The rows a table already holds each draw the added serial column's value, so the
-- declaration holds over them; the next row draws the next.
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('a'), ('b'), ('c');
ALTER TABLE acs ADD COLUMN id BIGSERIAL;
INSERT INTO acs (v) VALUES ('d');
-- @test
SELECT id, v FROM acs ORDER BY v;
