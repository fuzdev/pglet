-- @setup
CREATE TABLE nansort (id INT, x FLOAT8);
INSERT INTO nansort VALUES (1, 'nan'), (2, 1.0), (3, 2.0), (4, 'nan'), (5, -5.0);

-- @test
-- Postgres sorts float `NaN` as the greatest value, so under ASC the two NaN
-- rows land last; the `id` tiebreak keeps them in insertion order.
SELECT id, x FROM nansort ORDER BY x ASC, id ASC;
