-- A primary key over a date holding both infinities, read by a range above a day. The key
-- order itself is guarded by `infinity_date_primary_key_ordered_scan.sql`: a key encoder
-- with the two sentinels swapped leaves this read green, measured.
-- @profile-lock
-- @setup
CREATE TABLE infpk (d DATE PRIMARY KEY, v INT);
INSERT INTO infpk VALUES ('infinity', 1), ('-infinity', 2), ('2000-01-01', 3), ('9999-12-31', 4), ('0001-01-01', 5);
-- @test
SELECT d, v FROM infpk WHERE d > date '2000-01-01' ORDER BY d;
