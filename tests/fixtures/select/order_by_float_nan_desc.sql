-- @setup
CREATE TABLE nansort (id INT, x FLOAT8);
INSERT INTO nansort VALUES (1, 'nan'), (2, 1.0), (3, 2.0), (4, 'nan'), (5, -5.0);

-- @test
-- Under DESC the greatest-valued float `NaN` sorts first; the `id` tiebreak
-- keeps the two NaN rows in insertion order.
SELECT id, x FROM nansort ORDER BY x DESC, id ASC;
