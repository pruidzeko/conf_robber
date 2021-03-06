
use zlib_wrapper;
use file_system;

use GROUP_BLOKS_FLAG;
use structure::reader;
use structure::header::Header;
use structure::nested_block::NestedBlock;
use structure::attributes::{Attributes, GROUP_TYPE_MODULE, GROUP_TYPE_FORM, GROUP_TYPE_SIMPLY};
use structure::toc::TOC;
use std::path::Path;
use time;

use std::cell::RefCell;

#[derive(Copy, Clone, PartialEq)]
enum BlockType {
    FromCf, // необработанный блок данных
    Simply, // простой тип блока
    Multiple, // составной тип блока
}

// Блок конфигурационного файла включающий в себя две области:
// 1. область атрибутов
// 2. область данных
#[derive(Clone)]
pub struct Block {
    block_type: RefCell<BlockType>, // тип блока
    attrs: Attributes, // атрибуты блока
    source_data: RefCell<Vec<u8>>, // Исходные необработанные данные блока
    nested_blocks: RefCell<Vec<NestedBlock>>, // обработанные данные блока
}

impl Block {
    pub fn new(name: &str, data: &Vec<u8>) -> Block {

        let attrs = Attributes::new(0, 0, &String::from(name));

        Block {
            block_type: RefCell::new(BlockType::Simply),
            attrs: attrs.clone(),
            source_data: RefCell::new(Vec::new()),
            nested_blocks: RefCell::new(vec![NestedBlock::new(&attrs, data)]),
        }
    }

    // Создать новый блок на основании данных конфигурационного файла
    pub fn from_cf(source_data: &Vec<u8>, attrs_header: &Header, data_header: &Header) -> Block {

        trace!("Init block from cf");

        let (attrs, data) = get_attrs_and_data(source_data, attrs_header, data_header);

        trace!("Block name: {}", attrs.id());

        let retval = Block {
            block_type: RefCell::new(BlockType::FromCf),
            attrs: attrs,
            source_data: RefCell::new(data),
            nested_blocks: RefCell::new(Vec::new()),
        };
        trace!("-Init block from cf");

        retval
    }

    // Инициализировать блок из данных сохраненных в файлы
    pub fn from_file(path: &String) -> Block {

        trace!("Init block from the file: {}.", path);

        let mut block_type: BlockType = BlockType::Simply;
        let mut group_type: i32 = GROUP_TYPE_SIMPLY;
        let mut nested_blocks: Vec<NestedBlock> = Vec::new();
        let current_time = time::now().tm_nsec as u64;
        let block_id = file_system::file_name(path);

        if file_system::is_dir(path) {

            block_type = BlockType::Multiple;

            for (nested_block_id, nested_block_path) in &file_system::files_in_dir(path) {
                let nested_block_data = match file_system::read_file(nested_block_path) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("{}", e);
                        panic!("Error initialize multiple block from the file {}", e);
                    }
                };

                let nested_block_attrs =
                    Attributes::new(current_time, GROUP_TYPE_SIMPLY, &nested_block_id);
                nested_blocks.push(NestedBlock::new(&nested_block_attrs, &nested_block_data));

                match &*(*nested_block_id) {
                    "module" => group_type = GROUP_TYPE_MODULE,
                    "form" => group_type = GROUP_TYPE_FORM,
                    _ => continue,
                }
            }

        } else {
            let nested_block_data = match file_system::read_file(path) {
                Ok(v) => v,
                Err(e) => {
                    error!("{}", e);
                    panic!("Error initialize simple block from the file {}", e);
                }
            };

            let nested_block_attrs = Attributes::new(current_time, GROUP_TYPE_SIMPLY, &block_id);
            nested_blocks.push(NestedBlock::new(&nested_block_attrs, &nested_block_data));
        }

        let retval = Block {
            block_type: RefCell::new(block_type), // тип блока
            attrs: Attributes::new(current_time, group_type, &block_id), /* атрибуты блока */
            source_data: RefCell::new(Vec::new()), /* Исходные необработанные данные блока */
            nested_blocks: RefCell::new(nested_blocks), /* подчиненные блоки (если это составной блок) */
        };

        trace!("-Init block from the file.");

        return retval;
    }

    // Возвращает данные блока
    pub fn get_data(&self) -> Vec<NestedBlock> {
        self.decompress_data();
        if BlockType::Simply.ne(&self.block_type.borrow()) &&
           BlockType::Multiple.ne(&self.block_type.borrow()) {
            error!("Trying to getting an untreated data block.");
            panic!("Trying to getting an untreated data block.")
        }
        let mut retval: Vec<NestedBlock> = Vec::new();

        for sb in self.nested_blocks.borrow().iter() {
            retval.push(NestedBlock::new(&sb.attrs, &sb.data));
        }
        return retval;
    }

    // Устанавливает данные вложенного блока. Если блок простой, то у него есть только один вложенный блок
    // имя которого соотвествует имени самого блока
    pub fn set_data(&self, nested_block_name: &String, nested_block_data: &Vec<u8>) {

        if BlockType::Simply.ne(&self.block_type.borrow()) &&
           BlockType::Multiple.ne(&self.block_type.borrow()) {
            error!("Trying to getting an untreated data block.");
            panic!("Trying to getting an untreated data block.")
        }

        let mut is_find = false;

        for sb in self.nested_blocks.borrow_mut().iter_mut() {
            if sb.attrs.id().eq(&*(*nested_block_name)) {
                sb.data.clear();
                sb.data.extend_from_slice(&nested_block_data[..]);

                is_find = true;
            }
        }

        if !is_find {
            error!("Failed set new data. Block id - '{}' nested block name - '{}'",
                   &self.id(),
                   nested_block_name);
            panic!("Failed set new data. Block id - '{}' nested block name - '{}'",
                   &self.id(),
                   nested_block_name);
        }
    }

    // Получить данные блока для конфигурационного файла.
    pub fn for_cf(&self) -> (Vec<u8>, Vec<u8>) {

        let mut data: Vec<u8> = Vec::new();

        match *(self.block_type.borrow()) {
            BlockType::FromCf => {
                data.extend_from_slice(&(self.source_data.borrow()));
            }
            BlockType::Simply => {
                for sb in self.nested_blocks.borrow().iter() {
                    data.extend_from_slice(&sb.data);
                }
            }
            BlockType::Multiple => {
                let mut toc = TOC::new();
                let mut block_data: Vec<u8> = Vec::new();

                for sb in self.nested_blocks.borrow().iter() {
                    let sb_attrs_data = sb.attrs.for_cf();

                    let mut nested_block_data: Vec<u8> = Vec::new();
                    nested_block_data.extend_from_slice(&Header::for_cf(sb_attrs_data.len()));
                    nested_block_data.extend_from_slice(&sb_attrs_data);

                    let data_header_pos = nested_block_data.len() as i32;

                    nested_block_data.extend_from_slice(&Header::for_cf(sb.data.len()));
                    nested_block_data.extend_from_slice(&sb.data);

                    let header_sb_attr = Header::from_cf(&nested_block_data, 0);
                    let header_sb_data = Header::from_cf(&nested_block_data, data_header_pos);

                    toc.add(&header_sb_attr, &header_sb_data);
                    block_data.append(&mut nested_block_data);
                }

                data.extend_from_slice(&toc.for_cf());
                data.append(&mut block_data);
            }
        }

        if !data.is_empty() && BlockType::FromCf.ne(&self.block_type.borrow()) {
            data = zlib_wrapper::compress(&data);
        }

        return (self.attrs.for_cf(), data);
    }

    // Записать данные блока в файлы
    pub fn write_to_file(&self, path_to_dir: &String) {

        trace!("Write block to the file.");

        let nested_blocks = self.get_data();

        let path_to_block_dir: String = match *(self.block_type.borrow()) {
            BlockType::FromCf => {
                error!("Error recording unprocessed block.");
                panic!("Error recording unprocessed block.");
            }
            BlockType::Simply => path_to_dir.clone(),
            BlockType::Multiple => {
                let group_dir = Path::new(path_to_dir).join(self.id());
                let path = file_system::path_to_str(group_dir.as_path());
                path
            }
        };

        file_system::create_dir(&path_to_block_dir);

        for sb in nested_blocks {
            let id = sb.attrs.id();
            trace!("Nested block: {}", id);
            let file_name = Path::new(&path_to_block_dir).join(id);
            let file_name_str = file_system::path_to_str(file_name.as_path());

            match file_system::write_file(&*file_name_str, &sb.data) {
                Ok(_) => (),
                Err(e) => {
                    error!("Error writing block to the file: {}", e);
                    panic!("Error writing block to the file: {}", e);
                }
            };
        }

        trace!("-Write block to the file.");
    }

    // Получить наименование блока
    pub fn id<'a>(&'a self) -> &'a String {
        return self.attrs.id();
    }

    // Распаковывает данные блоков если они были получены из конфигурационного файла и не распакованы ранее.
    fn decompress_data(&self) {

        if BlockType::FromCf.ne(&self.block_type.borrow()) {
            return;
        }

        let mut block_data: Vec<u8> = Vec::new();
        if !self.source_data.borrow().is_empty() {
            let mut decompress_data = zlib_wrapper::decompress(&self.source_data.borrow());
            block_data.append(&mut decompress_data);
        }

        match TOC::from_cf(&block_data) {
            None => {
                *self.block_type.borrow_mut() = BlockType::Simply;
                self.nested_blocks.borrow_mut().push(NestedBlock::new(&self.attrs, &block_data));
            }
            Some(toc) => {
                if block_data[0..GROUP_BLOKS_FLAG.len()] != GROUP_BLOKS_FLAG {
                    error!("Flag of the group block is not found.");
                    panic!("Flag of the group block is not found.");
                }

                *self.block_type.borrow_mut() = BlockType::Multiple;
                self.nested_blocks.borrow_mut().clear();

                for address in toc.addresses() {
                    let attr_header = Header::from_cf(&block_data, address.attr_header_pos());
                    let data_header = Header::from_cf(&block_data, address.data_header_pos());

                    let (nested_block_attrs, nested_block_data) =
                        get_attrs_and_data(&block_data, &attr_header, &data_header);
                    self.nested_blocks
                        .borrow_mut()
                        .push(NestedBlock::new(&nested_block_attrs, &nested_block_data));
                }
            }
        }

        self.source_data.borrow_mut().clear();
    }

    // TODO

    // // Блок содержит данные формы
    // pub fn is_form(&mut self) -> bool {
    //     return self.check_block_type("form");
    // }

    // // Блок содержит данные модуля
    // pub fn is_module(&mut self) -> bool {
    //     return self.check_block_type("text");
    // }

    // // Проверить тип блока
    // fn check_block_type(&mut self, data_type: &str) -> bool {

    // self.decompress_data();

    //     match self.block_type {
    //         BlockType::FromCf => {
    //             error!("Trying to check the type of unprocessed block.");
    //             panic!("Trying to check the type of unprocessed block.");
    //         }
    //         BlockType::Simply => return false,
    //         BlockType::Multiple => {
    //             for sb in &self.nested_blocks {
    //                 if sb.attrs.name() == data_type {
    //                     return true;
    //                 };
    //             }
    //         }
    //     };

    // return false;

    // }
    //
}

// Получить данные атрибутов и данных блока на основании заголовков
fn get_attrs_and_data(source_data: &Vec<u8>,
                      attrs_header: &Header,
                      data_header: &Header)
                      -> (Attributes, Vec<u8>) {

    let attrs = get_attr(source_data, attrs_header);
    let mut block_data: Vec<u8> = Vec::new();
    let mut header = data_header.clone();

    loop {
        block_data.extend_from_slice(reader::get_region(source_data, &header));

        match header.next_header_position() {
            Some(pos) => header = Header::from_cf(source_data, pos),
            None => break,
        }

    }

    return (attrs, block_data);
}

// Получить данные атрибутов на основании заголовка
fn get_attr(source_data: &Vec<u8>, header: &Header) -> Attributes {

    let is_attrs_header = header.is_region_of_attrs(&source_data);
    if !is_attrs_header {
        error!("Failed initializing a header of attributes: position={}",
               header.region_position());
        panic!("Failed initializing a header of attributes: position={}",
               header.region_position());
    }

    let attrs_data = reader::get_region(source_data, header);

    return Attributes::from_cf(attrs_data);
}

#[test]
fn test_simple_block_from_cf() {

    let mut attrs: Vec<u8> = Vec::new();
    attrs.extend_from_slice(&[0xB0, 0x14, 0xC1, 0x30, 0x23, 0x42, 0x02, 0x00]);
    attrs.extend_from_slice(&[0xB0, 0x14, 0xC1, 0x30, 0x23, 0x42, 0x02, 0x00]);
    attrs.extend_from_slice(&[0xE4, 0x02, 0x00, 0x00]);
    attrs.extend_from_slice(&[0x72, 0x00, 0x6F, 0x00, 0x6F, 0x00, 0x74, 0x00]); // root
    attrs.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let attr_header = Header::for_cf(attrs.len());

    let data: Vec<u8> = vec![0xCA, 0x48, 0xCD, 0xC9, 0xC9, 0x07]; // hello
    let data_header = Header::for_cf(data.len());

    let mut block_data: Vec<u8> = Vec::new();
    block_data.extend_from_slice(&attr_header);
    block_data.extend_from_slice(&attrs);
    block_data.extend_from_slice(&data_header);
    block_data.extend_from_slice(&data);

    let header_attr_in_block = Header::from_cf(&block_data, 0);
    let header_data_in_block = Header::from_cf(&block_data,
                                               (attr_header.len() + attrs.len()) as i32);

    let test = Block::from_cf(&block_data, &header_attr_in_block, &header_data_in_block);

    let nested_blocks = test.get_data();
    let (new_block_attrs, new_block_data) = test.for_cf();

    assert_eq!(1, nested_blocks.len());
    assert_eq!(attrs, new_block_attrs);
    assert_eq!(data, new_block_data);
    assert_eq!("hello",
               String::from_utf8(nested_blocks.get(0).unwrap().data.clone()).unwrap());
}


#[test]
fn test_multi_block_from_cf() {
    use zlib_wrapper;

    let mut attrs: Vec<u8> = Vec::new();
    attrs.extend_from_slice(&[0xB0, 0x14, 0xC1, 0x30, 0x23, 0x42, 0x02, 0x00]);
    attrs.extend_from_slice(&[0xB0, 0x14, 0xC1, 0x30, 0x23, 0x42, 0x02, 0x00]);
    attrs.extend_from_slice(&[0xE4, 0x02, 0x00, 0x00]);
    attrs.extend_from_slice(&[0x72, 0x00, 0x6F, 0x00, 0x6F, 0x00, 0x74, 0x00]); // root
    attrs.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let attr_header = Header::for_cf(attrs.len());

    let data: Vec<u8> = vec![0x68, 0x65, 0x6C, 0x6C, 0x6F]; // hello
    let data_header = Header::for_cf(data.len());

    let mut block_data: Vec<u8> = Vec::new();
    block_data.extend_from_slice(&attr_header);
    block_data.extend_from_slice(&attrs);
    block_data.extend_from_slice(&data_header);
    block_data.extend_from_slice(&data);

    let header_attr_in_block = Header::from_cf(&block_data, 0);
    let header_data_in_block = Header::from_cf(&block_data,
                                               (attr_header.len() + attrs.len()) as i32);

    let mut toc = TOC::new();
    toc.add(&header_attr_in_block, &header_data_in_block);
    toc.add(&header_attr_in_block, &header_data_in_block);

    let mut data_multi_block = toc.for_cf();

    data_multi_block.extend_from_slice(&block_data);
    data_multi_block.extend_from_slice(&block_data);
    data_multi_block = zlib_wrapper::compress(&data_multi_block);

    let data_multi_block_header = Header::for_cf(data_multi_block.len());

    let mut multi_block: Vec<u8> = Vec::new();
    multi_block.extend_from_slice(&attr_header);
    multi_block.extend_from_slice(&attrs);
    multi_block.extend_from_slice(&data_multi_block_header);
    multi_block.extend_from_slice(&data_multi_block);

    let header_attr_in_multi_block = Header::from_cf(&multi_block, 0);
    let header_data_in_multi_block = Header::from_cf(&multi_block,
                                                     (attr_header.len() + attrs.len()) as i32);

    let test = Block::from_cf(&multi_block,
                              &header_attr_in_multi_block,
                              &header_data_in_multi_block);
    let nested_blocks = test.get_data();
    let (new_block_attrs, new_block_data) = test.for_cf();

    assert_eq!(2, nested_blocks.len());
    assert_eq!(attrs, new_block_attrs);
    assert_eq!(data_multi_block, new_block_data);
}
