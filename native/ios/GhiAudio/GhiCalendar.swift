// SPDX-License-Identifier: Apache-2.0
// The calendar (phase 17, P1): EventKit full access and the events around now,
// for Rust (C ABI in include/ghi_ios.h). Events are read on demand and handed
// over as JSON that Rust works out (names, addresses, call link) and never
// stores; only a recorded meeting keeps its own event's sealed facts. The
// user's own entry is left out of the people; the organizer comes first.

import EventKit
import Foundation

private struct RawPerson: Encodable {
    var name: String?
    var address: String
}

private struct RawEvent: Encodable {
    var id: String
    var title: String
    var startMs: Int64
    var endMs: Int64?
    var allDay: Bool
    var canceled: Bool
    var people: [RawPerson]
    var location: String?
    var url: String?
    var notes: String?
}

/// Most events and most of an event's notes handed to Rust (a call link is
/// looked for in the notes; nothing else reads them).
private let maxEvents = 2_000
private let maxNotesChars = 20_000

private func ms(_ d: Date) -> Int64 { Int64((d.timeIntervalSince1970 * 1000).rounded()) }

private func person(_ p: EKParticipant) -> RawPerson? {
    if p.isCurrentUser { return nil }
    return RawPerson(name: p.name, address: p.url.absoluteString)
}

private func raw(_ e: EKEvent) -> RawEvent? {
    guard let start = e.startDate else { return nil }
    let id = e.calendarItemExternalIdentifier ?? e.eventIdentifier ?? e.calendarItemIdentifier
    let people = ([e.organizer].compactMap { $0 } + (e.attendees ?? [])).compactMap(person)
    return RawEvent(
        id: id,
        title: e.title ?? "",
        startMs: ms(start),
        endMs: e.endDate.map(ms),
        allDay: e.isAllDay,
        canceled: e.status == .canceled,
        people: people,
        location: e.location,
        url: e.url?.absoluteString,
        notes: e.notes.map { String($0.prefix(maxNotesChars)) })
}

/// The one event store, made on first use; EventKit wants one that lives as
/// long as its events are used. Reads go one at a time under `lock`.
private final class Calendars {
    static let shared = Calendars()
    let lock = NSLock()
    private var store: EKEventStore?

    /// The store; the caller holds `lock`.
    func get() -> EKEventStore {
        if let store { return store }
        let s = EKEventStore()
        store = s
        return s
    }

    /// After access changed, the next call makes a new store; the caller holds `lock`.
    func forget() {
        store = nil
    }
}

private func realAccess() -> Int32 {
    switch EKEventStore.authorizationStatus(for: .event) {
    case .notDetermined: return 0
    case .fullAccess: return 1
    // Restricted, denied, or write-only (which cannot read events).
    default: return 2
    }
}

private func realEvents(_ from: Int64, _ to: Int64) -> String? {
    guard realAccess() == 1 else { return nil }
    let calendars = Calendars.shared
    calendars.lock.lock()
    defer { calendars.lock.unlock() }
    return autoreleasepool {
        let store = calendars.get()
        let predicate = store.predicateForEvents(
            withStart: Date(timeIntervalSince1970: Double(from) / 1000),
            end: Date(timeIntervalSince1970: Double(to) / 1000),
            calendars: nil)
        let events = store.events(matching: predicate).prefix(maxEvents).compactMap(raw)
        guard let data = try? JSONEncoder().encode(Array(events)) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

/// Calendar access: 0 not determined, 1 full access, 2 denied (or restricted,
/// or write-only, which cannot read events). Never prompts.
@_cdecl("ghi_swift_calendar_access")
public func ghiSwiftCalendarAccess() -> Int32 {
    #if GHI_TEST_HOOKS
    if let fake = GhiTestHooks.fakeCalendarAccess() { return fake }
    #endif
    return realAccess()
}

/// Shows the system prompt (first time only); poll `ghi_swift_calendar_access`.
@_cdecl("ghi_swift_request_calendar_access")
public func ghiSwiftRequestCalendarAccess() {
    #if GHI_TEST_HOOKS
    if GhiTestHooks.fakeCalendarRequest() { return }
    #endif
    guard realAccess() == 0 else { return }
    let calendars = Calendars.shared
    calendars.lock.lock()
    let store = calendars.get()
    calendars.lock.unlock()
    store.requestFullAccessToEvents { _, _ in
        calendars.lock.lock()
        calendars.forget()
        calendars.lock.unlock()
    }
}

/// The events starting in [from_ms, to_ms) as a JSON array (see `RawEvent` in
/// ghi-core's calendar module), in a heap string the caller frees with
/// `ghi_swift_string_free`. NULL without full access.
@_cdecl("ghi_swift_calendar_events")
public func ghiSwiftCalendarEvents(_ from: Int64, _ to: Int64) -> UnsafeMutablePointer<CChar>? {
    #if GHI_TEST_HOOKS
    if let fake = GhiTestHooks.fakeCalendarEvents(from, to) { return strdup(fake) }
    #endif
    return realEvents(from, to).flatMap { strdup($0) }
}

@_cdecl("ghi_swift_string_free")
public func ghiSwiftStringFree(_ s: UnsafeMutablePointer<CChar>?) {
    free(s)
}
