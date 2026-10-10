-- An `ADD COLUMN … DEFAULT now()` back-fill writes the instant the statement's own
-- `now()` reads — the transaction's start — through the closed default and through a
-- default that is an expression over it; a `statement_timestamp()` default back-fills
-- the `ALTER`'s own start, one value, later than the transaction's.
-- @setup
CREATE TABLE acn (id INT);
INSERT INTO acn VALUES (1), (2), (3), (4), (5);
ALTER TABLE acn ADD COLUMN nw TIMESTAMPTZ DEFAULT now();
ALTER TABLE acn ADD COLUMN bare TIMESTAMP DEFAULT now()::timestamp;
ALTER TABLE acn ADD COLUMN day DATE DEFAULT CURRENT_DATE;
ALTER TABLE acn ADD COLUMN st TIMESTAMPTZ DEFAULT statement_timestamp();
-- @test
SELECT count(*) AS backfilled_rows,
       sum(CASE WHEN nw = now() THEN 0 ELSE 1 END) AS not_the_transaction_start,
       sum(CASE WHEN bare = LOCALTIMESTAMP THEN 0 ELSE 1 END) AS expression_differs,
       sum(CASE WHEN day = CURRENT_DATE THEN 0 ELSE 1 END) AS day_differs,
       count(DISTINCT st) AS statement_values,
       min(st) > now() AS statement_after_transaction
FROM acn;
