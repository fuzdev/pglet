-- The accept side: rows of one length, each shorter than the table and under no column
-- list, are padded with their defaults on both engines — only a *difference* between the
-- rows' lengths is refused.
-- @setup
CREATE TABLE vls (a INT, b INT DEFAULT 7, c TEXT DEFAULT 'd');

-- @test
INSERT INTO vls VALUES (1), (2), (DEFAULT) RETURNING a, b, c;
