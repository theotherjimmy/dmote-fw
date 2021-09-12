//! Keyboard HID device implementation.

use crate::hid::{HidDevice, Protocol, ReportType, Subclass};
use crate::key_code::KbHidReport;

const REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00, 0x25, 0x01,
    0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x03, 0x95, 0x05, 0x75, 0x01,
    0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x03, 0x95, 0x06,
    0x75, 0x08, 0x15, 0x00, 0x25, 0xFB, 0x05, 0x07, 0x19, 0x00, 0x29, 0xFB, 0x81, 0x00, 0x09, 0x03,
    0x75, 0x08, 0x95, 0x40, 0xB1, 0x02, 0xC0,
];

/// A keyboard HID device.
#[derive(Default)]
pub struct Keyboard {
    pub report: KbHidReport,
}

impl HidDevice for Keyboard {
    fn subclass(&self) -> Subclass {
        Subclass::BootInterface
    }

    fn protocol(&self) -> Protocol {
        Protocol::Keyboard
    }

    fn report_descriptor(&self) -> &[u8] {
        REPORT_DESCRIPTOR
    }

    fn get_report(&mut self, report_type: ReportType, _report_id: u8) -> Result<&[u8], ()> {
        match report_type {
            ReportType::Input => Ok(self.report.as_bytes()),
            _ => Err(()),
        }
    }

    fn set_report(
        &mut self,
        report_type: ReportType,
        report_id: u8,
        data: &[u8],
    ) -> Result<(), ()> {
        if report_type == ReportType::Output && report_id == 0 && data.len() == 1 {
            return Ok(());
        }
        Err(())
    }
}
