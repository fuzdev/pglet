-- COPY ... FROM STDIN ingesting every spine-relevant type (int, text, bool, text[],
-- uuid, jsonb, timestamptz, bytea) plus a NULL row, then verify the rows landed
-- identically to Postgres.
-- @setup
CREATE TABLE t (
  id int,
  name text,
  active bool,
  tags text[],
  uid uuid,
  meta jsonb,
  ts timestamptz,
  payload bytea
);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	alice	t	{a,b}	11111111-1111-1111-1111-111111111111	{"k": 1}	2024-06-15 12:30:45.123+00	\\xdeadbeef
2	bob	f	{c,d e}	22222222-2222-2222-2222-222222222222	{"k": 2}	2024-01-02 03:04:05+00	\\x00ff
3	\N	\N	\N	\N	\N	\N	\N
\.
-- @verify
SELECT id, name, active, tags, uid, meta, ts, payload FROM t ORDER BY id;
