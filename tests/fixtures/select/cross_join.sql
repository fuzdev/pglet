-- @setup
CREATE TABLE colors (c TEXT);
CREATE TABLE sizes (s TEXT);
INSERT INTO colors VALUES ('red'), ('blue');
INSERT INTO sizes VALUES ('S'), ('M'), ('L');

-- @test
SELECT c, s FROM colors CROSS JOIN sizes ORDER BY c, s;
