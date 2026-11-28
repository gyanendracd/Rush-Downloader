/* The speed limit, where it is shown outside Settings: under the speed in
 * the sidebar's transfer summary (src/views.tsx, TransferSummary). */
export const speed = {
  /** Under the speed while a limit is on, e.g. "Limited to 4.0 MB/s". */
  limited: (rate: string) => `Limited to ${rate}`,
  limitedTitle: "Every download together stays under this speed. Change it under Settings, Performance, Max download speed.",
};
