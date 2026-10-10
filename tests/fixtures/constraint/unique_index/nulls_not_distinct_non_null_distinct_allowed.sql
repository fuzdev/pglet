-- `NULLS NOT DISTINCT` only changes the NULL-key rule; distinct non-null values
-- still insert cleanly (the unique index behaves normally for non-nulls).
-- @setup
CREATE TABLE nnd_distinct (id INT, tag TEXT);
CREATE UNIQUE INDEX ux_nnd_distinct ON nnd_distinct (tag) NULLS NOT DISTINCT;
INSERT INTO nnd_distinct VALUES (1, 'a'), (2, 'b'), (3, 'c');

-- @test
SELECT id, tag FROM nnd_distinct ORDER BY id;
