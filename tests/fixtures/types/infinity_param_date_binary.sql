-- A binary `date` parameter holding the `infinity` sentinel, bound over the extended
-- protocol and matched against a stored `infinity`.
-- @setup
CREATE TABLE infpd (id INT, d DATE);
INSERT INTO infpd VALUES (1, 'infinity'), (2, '-infinity'), (3, '2000-01-01');
-- @param $1 date infinity
-- @test
SELECT id FROM infpd WHERE d = $1;
