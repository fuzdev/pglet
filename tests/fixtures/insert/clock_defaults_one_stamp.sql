-- Every row one `INSERT` writes takes one transaction-start instant, whichever door
-- the value comes through: two `DEFAULT now()`-family columns of one row, the rows of a
-- multi-row `VALUES`, the rows of an `INSERT … SELECT`, and a `now()` the statement
-- writes beside the defaults. A `DEFAULT statement_timestamp()` takes one instant a
-- statement, and a `DEFAULT clock_timestamp()` is read per row and is no earlier.
--
-- The fixture runs inside the harness's block, so the three statements share the
-- transaction's start and each has its own statement start.
-- @setup
CREATE TABLE cdo_d (x INT);
INSERT INTO cdo_d VALUES (0), (1), (2), (3), (4), (5), (6), (7);
CREATE TABLE cdo (
    id INT,
    created TIMESTAMPTZ DEFAULT now(),
    updated TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    via_tx TIMESTAMPTZ DEFAULT transaction_timestamp(),
    local_ts TIMESTAMP DEFAULT LOCALTIMESTAMP,
    day DATE DEFAULT CURRENT_DATE,
    stmt TIMESTAMPTZ DEFAULT statement_timestamp(),
    clk TIMESTAMPTZ DEFAULT clock_timestamp(),
    written TIMESTAMPTZ,
    src INT
);
INSERT INTO cdo (id, written, src) VALUES (1, now(), 1), (2, now(), 1), (3, now(), 1);
INSERT INTO cdo (id, written, src) SELECT 10 + a.x * 64 + b.x * 8 + c.x, now(), 2 FROM cdo_d a, cdo_d b, cdo_d c;
INSERT INTO cdo (id, written, src) VALUES (1000, CURRENT_TIMESTAMP, 3);
-- @test
SELECT count(*) AS written_rows,
       count(DISTINCT created) AS created_values,
       count(DISTINCT updated) AS updated_values,
       count(DISTINCT via_tx) AS transaction_values,
       count(DISTINCT local_ts) AS local_values,
       count(DISTINCT day) AS day_values,
       count(DISTINCT written) AS written_values,
       count(DISTINCT stmt) AS statement_values,
       sum(CASE WHEN created = updated AND created = via_tx AND created = written THEN 0 ELSE 1 END) AS columns_differing,
       sum(CASE WHEN created::timestamp = local_ts AND created::date = day THEN 0 ELSE 1 END) AS casts_differing,
       sum(CASE WHEN created = now() THEN 0 ELSE 1 END) AS not_this_transaction,
       sum(CASE WHEN created <= stmt AND stmt <= clk THEN 0 ELSE 1 END) AS out_of_order
FROM cdo;
