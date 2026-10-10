-- @setup
CREATE TABLE colors (c TEXT);
CREATE TABLE sizes (s TEXT);
INSERT INTO colors VALUES ('red'), ('blue');
INSERT INTO sizes VALUES ('S'), ('M');

-- @test
SELECT c, s FROM colors, sizes ORDER BY c, s;
