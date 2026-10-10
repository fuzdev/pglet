-- COPY ... FROM STDIN filling `DEFAULT now()`-family columns: every ingested row takes
-- one transaction-start instant and one statement-start instant, as an `INSERT`'s rows
-- do; a `DEFAULT clock_timestamp()` is read per row and is no earlier.
-- @setup
CREATE TABLE cfn (id int, created timestamptz DEFAULT now(), updated timestamptz DEFAULT CURRENT_TIMESTAMP, stmt timestamptz DEFAULT statement_timestamp(), clk timestamptz DEFAULT clock_timestamp(), day date DEFAULT CURRENT_DATE);
-- @copy-from
COPY cfn (id) FROM STDIN;
-- @data
1
2
3
4
5
6
-- @verify
SELECT count(*) AS n, count(DISTINCT created) AS created_values, count(DISTINCT stmt) AS statement_values, count(DISTINCT day) AS day_values, sum(CASE WHEN created = updated AND created <= stmt AND stmt <= clk THEN 0 ELSE 1 END) AS bad FROM cfn;
