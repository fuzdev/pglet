-- A statement with bound parameters reads one transaction start and one statement start,
-- as a literal statement does: the stamps it writes, its `DEFAULT`, and its `RETURNING`
-- agree.
-- @setup
CREATE TABLE cbs (id INT, a TIMESTAMPTZ, b TIMESTAMPTZ, c TIMESTAMPTZ DEFAULT now(), s TIMESTAMPTZ DEFAULT statement_timestamp());
-- @param $1 int4 7
-- @param $2 int4 8
-- @test
INSERT INTO cbs (id, a, b) VALUES ($1, now(), now()), ($2, CURRENT_TIMESTAMP, transaction_timestamp())
RETURNING id, a = b AS written_agree, a = c AS default_agrees, a = now() AS returning_agrees,
          s = statement_timestamp() AS statement_agrees, a <= s AS transaction_before_statement;
