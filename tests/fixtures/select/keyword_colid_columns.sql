-- @setup
-- Every Postgres `ColId` keyword pglet used to over-reserve, exercised as an
-- ordinary column name — in DDL (`CREATE TABLE`), in `INSERT`, and in the
-- `SELECT` projection. These are the UNRESERVED and COL_NAME classes
-- (`kwlist.h`), which Postgres admits as bare identifiers; pglet must too. (The
-- TYPE_FUNC_NAME words `is`/`like`/`left`/… are *not* here — Postgres rejects
-- those as bare columns, and so does pglet; see `keyword_typefunc_reject.sql`.)
CREATE TABLE kw (
    insert INT, update INT, delete INT, set INT, by INT, drop INT,
    alter INT, index INT, truncate INT, nulls INT, first INT, last INT,
    conflict INT, nothing INT, begin INT, commit INT, rollback INT, start INT,
    transaction INT, recursive INT, values INT, between INT
);
INSERT INTO kw VALUES (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22);

-- @test
SELECT insert, update, delete, set, by, drop, alter, index, truncate, nulls,
       first, last, conflict, nothing, begin, commit, rollback, start,
       transaction, recursive, values, between
FROM kw
WHERE between > 0
ORDER BY values DESC;
