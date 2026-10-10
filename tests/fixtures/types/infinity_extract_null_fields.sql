-- `EXTRACT` of a cycling field from an infinity is NULL, and so is a time-of-day or zone
-- field of an infinite timestamp. Every cycling field is also asked of a `date`, whose
-- `EXTRACT` refuses a field outside that arm's list (`0A000` for a time-of-day field), so a
-- field dropped from the list reddens here; a timestamp source would answer NULL either way.
-- @test
SELECT extract(day FROM date 'infinity') AS a, extract(month FROM timestamp '-infinity') AS b,
       extract(week FROM date '-infinity') AS c, extract(doy FROM timestamptz 'infinity') AS d,
       extract(second FROM timestamp 'infinity') AS e, extract(timezone FROM timestamp '-infinity') AS f,
       extract(month FROM date 'infinity') AS g, extract(quarter FROM date '-infinity') AS h,
       extract(dow FROM date 'infinity') AS i, extract(isodow FROM date '-infinity') AS j,
       extract(doy FROM date 'infinity') AS k;
