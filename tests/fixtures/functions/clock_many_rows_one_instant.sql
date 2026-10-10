-- A predicate that reads the transaction clock twice per row holds for every row: the
-- two reads are one instant however many rows the scan visits. The statement clock is
-- the same, and each row's wall-clock read is no earlier than either.
-- @setup
CREATE TABLE cmr_d (x INT);
INSERT INTO cmr_d VALUES (0), (1), (2), (3), (4), (5), (6), (7);
CREATE TABLE cmr (id INT);
INSERT INTO cmr SELECT a.x * 512 + b.x * 64 + c.x * 8 + d.x FROM cmr_d a, cmr_d b, cmr_d c, cmr_d d;
-- @test
SELECT count(*) AS scanned,
       sum(CASE WHEN now() = now() THEN 1 ELSE 0 END) AS transaction_reads_agree,
       sum(CASE WHEN LOCALTIMESTAMP = LOCALTIMESTAMP THEN 1 ELSE 0 END) AS local_reads_agree,
       sum(CASE WHEN statement_timestamp() = statement_timestamp() THEN 1 ELSE 0 END) AS statement_reads_agree,
       sum(CASE WHEN now() > now() OR now() < now() THEN 1 ELSE 0 END) AS transaction_reads_differ,
       sum(CASE WHEN statement_timestamp() <= clock_timestamp() THEN 1 ELSE 0 END) AS clock_not_before_statement
FROM cmr
WHERE now() = now() AND CURRENT_TIMESTAMP = transaction_timestamp();
