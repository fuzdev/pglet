-- The "no end date" column: `NOT NULL DEFAULT 'infinity'`, read back as still valid.
-- @setup
CREATE TABLE infvalid (id INT, valid_to DATE NOT NULL DEFAULT 'infinity', ends TIMESTAMPTZ DEFAULT 'infinity');
INSERT INTO infvalid (id) VALUES (1);
INSERT INTO infvalid VALUES (2, '2001-01-01', '2001-01-01 00:00:00+00');
-- @test
SELECT id, valid_to, ends, valid_to > CURRENT_DATE AS open_date, ends > now() AS open_ts FROM infvalid ORDER BY id;
