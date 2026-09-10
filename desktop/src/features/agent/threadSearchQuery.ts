/** One Han character is meaningful; other queries need two characters. */
export function isThreadSearchQuery(query: string): boolean {
  return /\p{Script=Han}/u.test(query)
    || (query.trim().length > 0 && Array.from(query).length >= 2);
}
