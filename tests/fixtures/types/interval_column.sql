-- An INTERVAL column stores and reloads: the 3-field value round-trips through the
-- wire (INSERT then SELECT), including negative and month/day/time mixes.
-- @setup
CREATE TABLE iv_store (id INT, span INTERVAL);
INSERT INTO iv_store VALUES (1, INTERVAL '1 year 2 mons 3 days 04:05:06'),
                            (2, INTERVAL '30 days'),
                            (3, INTERVAL '-2 hours');
-- @test
SELECT id, span FROM iv_store ORDER BY id ASC;
