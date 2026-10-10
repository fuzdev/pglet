-- @setup
CREATE TABLE codes (id INT, code TEXT);
INSERT INTO codes VALUES (1, 'AB'), (2, 'ABC'), (3, 'ABCD'), (4, 'A');

-- @test
SELECT * FROM codes WHERE code LIKE 'A_' ORDER BY id;
