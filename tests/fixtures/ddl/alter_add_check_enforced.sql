-- @expect-error: an ALTER-added CHECK rejects a violating row
-- @setup
CREATE TABLE isc_enf (a INT, b INT);
ALTER TABLE isc_enf ADD CONSTRAINT isc_enf_ck CHECK (a < b);

-- @test
INSERT INTO isc_enf VALUES (5, 2);
