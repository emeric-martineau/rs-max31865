//! A generic driver for the MAX31865 RTD to Digital converter
//!
//! # References
//! - Datasheet: https://datasheets.maximintegrated.com/en/ds/MAX31865.pdf

#![cfg_attr(not(test), no_std)]
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::spi::{Mode, Phase, Polarity, SpiBus};

#[cfg(feature = "doc")]
pub mod examples;

pub const MODE: Mode = Mode {
    phase: Phase::CaptureOnSecondTransition,
    polarity: Polarity::IdleHigh,
};

pub mod temp_conversion;

pub enum FilterMode {
    Filter60Hz = 0,
    Filter50Hz = 1,
}

pub enum SensorType {
    TwoOrFourWire = 0,
    ThreeWire = 1,
}

pub struct Max31865<SPI, NCS, RDY> {
    spi: SPI,
    ncs: NCS,
    rdy: RDY,
    calibration: u32,
}

#[derive(Debug)]
pub enum Error {
    SpiErrorRead,
    SpiErrorWrite,
    SpiErrorTransfert,
    PinError,
}

impl<SPI, NCS, RDY> Max31865<SPI, NCS, RDY>
where
    SPI: SpiBus<u8>,
    NCS: OutputPin,
    RDY: InputPin,
{
    /// Create a new MAX31865 module.
    ///
    /// # Arguments
    ///
    /// * `spi` - The SPI module to communicate on.
    /// * `ncs` - The chip select pin which should be set to a push pull output
    ///           pin.
    /// * `rdy` - The ready pin which is set low by the MAX31865 controller
    ///           whenever it has finished converting the output.
    ///
    pub fn new(
        spi: SPI,
        mut ncs: NCS,
        rdy: RDY,
    ) -> Result<Max31865<SPI, NCS, RDY>, Error> {
        let default_calib = 40000;

        ncs.set_high().map_err(|_| Error::PinError)?;
        let max31865 = Max31865 {
            spi,
            ncs,
            rdy,
            calibration: default_calib, /* value in ohms multiplied by 100 */
        };

        Ok(max31865)
    }

    /// Updates the devices configuration.
    ///
    /// # Arguments
    /// * `vbias` - Set to `true` to enable V_BIAS voltage, which is required to
    ///             correctly perform conversion.Clone
    /// * `conversion_mode` - `true` to automatically perform conversion,
    ///                       otherwise normally off.
    /// * `one_shot` - Only perform detection once if set to `true`, otherwise
    ///             repeats conversion.
    /// * `sensor_type` - Define whether a two, three or four wire sensor is
    ///                   used.
    /// * `filter_mode` - Specify the mains frequency that should be used to
    ///                   filter out noise, e.g. 50Hz in Europe.
    ///
    /// # Remarks
    ///
    /// This will update the configuration register of the MAX31865 register. If
    /// the device doesn't properly react to this, add a delay after calling
    /// `new` to increase the time that the chip select line is set high.
    ///
    /// *Note*: The correct sensor configuration also requires changes to the
    /// PCB! Make sure to read the data sheet concerning this.
    pub fn configure(
        &mut self,
        vbias: bool,
        conversion_mode: bool,
        one_shot: bool,
        sensor_type: SensorType,
        filter_mode: FilterMode,
    ) -> Result<(), Error> {
        let conf: u8 = ((vbias as u8) << 7)
            | ((conversion_mode as u8) << 6)
            | ((one_shot as u8) << 5)
            | ((sensor_type as u8) << 4)
            | (filter_mode as u8);

        self.write(Register::CONFIG, conf)?;

        Ok(())
    }

    /// Set the calibration reference resistance. This can be used to calibrate
    /// inaccuracies of both the reference resistor and the PT100 element.
    ///
    /// # Arguments
    ///
    /// * `calib` - A 32 bit integer specifying the reference resistance in ohms
    ///             multiplied by 100, e.g. `40000` for 400 Ohms
    ///
    /// # Remarks
    ///
    /// You can perform calibration by putting the sensor in boiling (100
    /// degrees Celsius) water and then measuring the raw value using
    /// `read_raw`. Calculate `calib` as `(13851 << 15) / raw >> 1`.
    pub fn set_calibration(&mut self, calib: u32) {
        self.calibration = calib;
    }

    /// Read the raw resistance value.
    ///
    /// # Remarks
    ///
    /// The output value is the value in Ohms multiplied by 100.
    pub fn read_ohms(&mut self) -> Result<u32, Error> {
        let raw = self.read_raw()?;
        let ohms = ((raw >> 1) as u32 * self.calibration) >> 15;

        Ok(ohms)
    }

    /// Read the raw resistance value and then perform conversion to degrees Celsius.
    ///
    /// # Remarks
    ///
    /// The output value is the value in degrees Celsius multiplied by 100.
    pub fn read_default_conversion(&mut self) -> Result<i32, Error> {
        let ohms = self.read_ohms()?;
        let temp = temp_conversion::LOOKUP_VEC_PT100.lookup_temperature(ohms as i32);

        Ok(temp)
    }

    /// Read the raw RTD value.
    ///
    /// # Remarks
    ///
    /// The raw value is the value of the combined MSB and LSB registers.
    /// The first 15 bits specify the ohmic value in relation to the reference
    /// resistor (i.e. 2^15 - 1 would be the exact same resistance as the reference
    /// resistor). See manual for further information.
    /// The last bit specifies if the conversion was successful.
    pub fn read_raw(&mut self) -> Result<u16, Error> {
        let msb: u16 = self.read(Register::RTD_MSB)? as u16;
        let lsb: u16 = self.read(Register::RTD_LSB)? as u16;

        Ok((msb << 8) | lsb)
    }

    /// Determine if a new conversion is available
    ///
    /// # Remarks
    ///
    /// When the module is finished converting the temperature it sets the
    /// ready pin to low. It is automatically returned to high upon reading the
    /// RTD registers.
    pub fn is_ready(&mut self) -> Result<bool, RDY::Error> {
        self.rdy.is_low()
    }

    fn read(&mut self, reg: Register) -> Result<u8, Error> {
        let buffer: [u8; 2] = self.read_two(reg)?;
        Ok(buffer[1])
    }

    fn read_two(&mut self, reg: Register) -> Result<[u8; 2], Error> {
        let mut read_buffer = [0u8; 2];
        let mut write_buffer = [0u8; 1];

        write_buffer[0] = reg.read_address();
        self.ncs.set_low().map_err(|_| Error::PinError)?;
        self.spi
            .transfer(&mut read_buffer, &mut write_buffer)
            .map_err(|_| Error::SpiErrorTransfert)?;
        self.ncs.set_high().map_err(|_| Error::PinError)?;

        Ok(read_buffer)
    }

    fn write(&mut self, reg: Register, val: u8) -> Result<(), Error> {
        self.ncs.set_low().map_err(|_| Error::PinError)?;
        self.spi
            .write(&[reg.write_address(), val])
            .map_err(|_| Error::SpiErrorWrite)?;
        self.ncs.set_high().map_err(|_| Error::PinError)?;
        Ok(())
    }
}

#[allow(non_camel_case_types)]
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum Register {
    CONFIG = 0x00,
    RTD_MSB = 0x01,
    RTD_LSB = 0x02,
    HIGH_FAULT_THRESHOLD_MSB = 0x03,
    HIGH_FAULT_THRESHOLD_LSB = 0x04,
    LOW_FAULT_THRESHOLD_MSB = 0x05,
    LOW_FAULT_THRESHOLD_LSB = 0x06,
    FAULT_STATUS = 0x07,
}

const R: u8 = 0 << 7;
const W: u8 = 1 << 7;

impl Register {
    fn read_address(&self) -> u8 {
        *self as u8 | R
    }

    fn write_address(&self) -> u8 {
        *self as u8 | W
    }
}
