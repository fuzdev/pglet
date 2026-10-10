-- A string literal takes the list's common type, not the column's: beside a `timestamp`
-- item under a `date` column it is a `timestamp`, time of day and all.
-- @setup
CREATE TABLE inl_t (id INT, dt DATE);
INSERT INTO inl_t VALUES (1, '2020-01-01'), (2, '2020-01-02'), (3, '2020-01-03');
-- @test
SELECT id FROM inl_t
WHERE dt IN ('2020-01-01 00:00:00', '2020-01-02 10:00:00'::timestamp, '2020-01-03'::timestamp) ORDER BY id;
