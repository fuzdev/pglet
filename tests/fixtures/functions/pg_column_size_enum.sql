-- An enum column is stored as its member's four-byte OID, so its size is 4 whatever the
-- label's length.
-- @setup
CREATE TYPE pcs_mood AS ENUM ('ok', 'very_unhappy_indeed');
CREATE TABLE pcs_e (m pcs_mood);
INSERT INTO pcs_e VALUES ('ok'), ('very_unhappy_indeed');
-- @test
SELECT m::text AS label, pg_column_size(m) AS size FROM pcs_e ORDER BY 1;
